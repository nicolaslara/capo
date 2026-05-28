use capo_state::{
    AdapterDispatchExecutionProjection, AdapterDispatchGateProjection,
    AdapterDispatchPlanProjection, AdapterDispatchPromptMaterializationProjection,
    AdapterDispatchPromptSourceProjection, EventKind, NewEvent, ProjectionRecord, RedactionState,
    RunProjection,
};

use crate::util::stable_hash;
use crate::{CapoServer, ServerClientOrigin, ServerError, ServerResult};

pub(crate) struct DispatchExecutionOutcome<'a> {
    pub(crate) status: &'a str,
    pub(crate) provider_cli_executed: bool,
    pub(crate) runtime_process_ref: Option<String>,
    pub(crate) exit_code: Option<i64>,
    pub(crate) stdout_artifact_id: Option<String>,
    pub(crate) stderr_artifact_id: Option<String>,
    pub(crate) credential_scan_status: &'a str,
    pub(crate) raw_output_policy: &'a str,
    pub(crate) reason_codes: &'a str,
}

pub(crate) struct DispatchReplayMetadata<'a> {
    pub(crate) provider_cli_executed: bool,
    pub(crate) raw_content_policy: &'a str,
}

impl CapoServer {
    pub(crate) fn dispatch_plan_with_prompt(
        &self,
        dispatch_plan_id: &str,
    ) -> ServerResult<(
        AdapterDispatchPlanProjection,
        AdapterDispatchPromptSourceProjection,
    )> {
        let plan = self
            .controller
            .state()
            .adapter_dispatch_plans(&self.project_id)
            .map_err(ServerError::State)?
            .into_iter()
            .find(|plan| plan.dispatch_plan_id == dispatch_plan_id)
            .ok_or_else(|| ServerError::UnknownDispatchPlan {
                dispatch_plan_id: dispatch_plan_id.to_string(),
            })?;
        let prompt_source = self
            .controller
            .state()
            .adapter_dispatch_prompt_sources(&self.project_id)
            .map_err(ServerError::State)?
            .into_iter()
            .rev()
            .find(|source| source.dispatch_plan_id == dispatch_plan_id)
            .ok_or_else(|| {
                ServerError::AdapterFixture(format!(
                    "dispatch plan has no prompt source: {dispatch_plan_id}"
                ))
            })?;
        Ok((plan, prompt_source))
    }

    pub(crate) fn dispatch_gate_for_plan(
        &self,
        plan: &AdapterDispatchPlanProjection,
    ) -> AdapterDispatchGateProjection {
        let mut reasons = Vec::new();
        if plan.status != "planned" {
            reasons.push(format!("dispatch_plan_status:{}", plan.status));
        }
        if plan.runtime_prompt_policy != "not_rendered" {
            reasons.push("runtime_prompt_policy_not_redacted".to_string());
        }
        if plan.provider_cli_executed {
            reasons.push("provider_cli_already_executed".to_string());
        }
        if plan.request_env_count == 0 {
            reasons.push("missing_deterministic_fixture_opt_in".to_string());
        }
        if plan.runtime_program != "deterministic-fixture-runtime" {
            reasons.push("unsafe_runtime_program".to_string());
        }
        if plan.stdout_format != "jsonl" {
            reasons.push("unsupported_stdout_format".to_string());
        }
        if plan.stderr_policy != "bounded_redacted_artifact" || plan.redaction_rule_count == 0 {
            reasons.push("missing_artifact_scan_policy".to_string());
        }
        if plan.artifact_root.trim().is_empty() {
            reasons.push("missing_artifact_root".to_string());
        }
        if reasons.is_empty() {
            reasons.push("deterministic_fixture_dispatch_allowed".to_string());
        }
        let allowed = reasons == ["deterministic_fixture_dispatch_allowed"];
        AdapterDispatchGateProjection {
            dispatch_gate_id: format!(
                "server-dispatch-gate-{}-{}",
                stable_hash(plan.dispatch_plan_id.as_bytes()),
                stable_hash(reasons.join(",").as_bytes())
            ),
            project_id: plan.project_id.clone(),
            dispatch_plan_id: plan.dispatch_plan_id.clone(),
            adapter_kind: plan.adapter_kind.clone(),
            provider_cli_execution_allowed: allowed,
            status: if allowed {
                "ready_for_deterministic_execution".to_string()
            } else {
                "blocked".to_string()
            },
            required_dogfood_gate: "deterministic_fixture_path".to_string(),
            reason_codes: reasons.join(","),
            provider_cli_executed: false,
            runtime_prompt_policy: plan.runtime_prompt_policy.clone(),
            updated_sequence: 0,
        }
    }

    pub(crate) fn dispatch_prompt_materialization(
        &self,
        source: &AdapterDispatchPromptSourceProjection,
    ) -> AdapterDispatchPromptMaterializationProjection {
        AdapterDispatchPromptMaterializationProjection {
            materialization_id: format!(
                "server-dispatch-prompt-materialization-{}",
                stable_hash(source.prompt_source_id.as_bytes())
            ),
            project_id: source.project_id.clone(),
            dispatch_plan_id: source.dispatch_plan_id.clone(),
            prompt_source_id: source.prompt_source_id.clone(),
            source_kind: source.source_kind.clone(),
            source_ref: source.source_ref.clone(),
            expected_source_hash: source.source_hash.clone(),
            observed_source_hash: source.source_hash.clone(),
            expected_prompt_hash: source.prompt_hash.clone(),
            materialized_prompt_hash: Some(source.prompt_hash.clone()),
            status: "ready_without_rendering_prompt".to_string(),
            raw_prompt_policy: "not_rendered".to_string(),
            reason_codes: "server_goal_hash_matches".to_string(),
            updated_sequence: 0,
        }
    }

    pub(crate) fn dispatch_execution_request(
        &self,
        plan: &AdapterDispatchPlanProjection,
        gate: &AdapterDispatchGateProjection,
    ) -> capo_state::AdapterDispatchExecutionRequestProjection {
        capo_state::AdapterDispatchExecutionRequestProjection {
            execution_request_id: format!(
                "server-dispatch-execution-request-{}-{}",
                stable_hash(plan.dispatch_plan_id.as_bytes()),
                stable_hash(gate.dispatch_gate_id.as_bytes())
            ),
            project_id: plan.project_id.clone(),
            dispatch_plan_id: plan.dispatch_plan_id.clone(),
            dispatch_gate_id: gate.dispatch_gate_id.clone(),
            adapter_kind: plan.adapter_kind.clone(),
            provider_cli_execution_allowed: gate.provider_cli_execution_allowed,
            provider_cli_executed: false,
            status: if gate.provider_cli_execution_allowed {
                "ready_for_deterministic_fixture_run".to_string()
            } else {
                "blocked_by_dispatch_gate".to_string()
            },
            opt_in_env: format!(
                "CAPO_SERVER_DETERMINISTIC_{}",
                plan.adapter_kind.to_uppercase()
            ),
            runtime_prompt_policy: plan.runtime_prompt_policy.clone(),
            reason_codes: gate.reason_codes.clone(),
            updated_sequence: 0,
        }
    }

    pub(crate) fn latest_dispatch_gate(
        &self,
        dispatch_plan_id: &str,
    ) -> ServerResult<AdapterDispatchGateProjection> {
        self.controller
            .state()
            .adapter_dispatch_gates(&self.project_id)
            .map_err(ServerError::State)?
            .into_iter()
            .rev()
            .find(|gate| gate.dispatch_plan_id == dispatch_plan_id)
            .ok_or_else(|| {
                ServerError::AdapterFixture(format!(
                    "dispatch plan has no gate: {dispatch_plan_id}"
                ))
            })
    }

    pub(crate) fn latest_execution_request(
        &self,
        dispatch_plan_id: &str,
    ) -> ServerResult<capo_state::AdapterDispatchExecutionRequestProjection> {
        self.controller
            .state()
            .adapter_dispatch_execution_requests(&self.project_id)
            .map_err(ServerError::State)?
            .into_iter()
            .rev()
            .find(|request| request.dispatch_plan_id == dispatch_plan_id)
            .ok_or_else(|| {
                ServerError::AdapterFixture(format!(
                    "dispatch plan has no execution request: {dispatch_plan_id}"
                ))
            })
    }

    pub(crate) fn latest_dispatch_execution(
        &self,
        dispatch_plan_id: &str,
    ) -> ServerResult<Option<AdapterDispatchExecutionProjection>> {
        Ok(self
            .controller
            .state()
            .adapter_dispatch_executions(&self.project_id)
            .map_err(ServerError::State)?
            .into_iter()
            .rev()
            .find(|execution| execution.dispatch_plan_id == dispatch_plan_id))
    }

    pub(crate) fn reject_changed_dispatch_fixture(
        &self,
        dispatch_plan_id: &str,
        fixture_hash: &str,
    ) -> ServerResult<()> {
        let existing = self
            .controller
            .state()
            .adapter_dispatch_replays(&self.project_id)
            .map_err(ServerError::State)?
            .into_iter()
            .find(|replay| replay.dispatch_plan_id == dispatch_plan_id);
        if let Some(existing) = existing
            && existing.fixture_hash != fixture_hash
        {
            return Err(ServerError::AdapterFixture(format!(
                "dispatch plan already ran with fixture hash {}; refusing changed fixture hash {fixture_hash}",
                existing.fixture_hash
            )));
        }
        Ok(())
    }

    pub(crate) fn dispatch_plan_turn_id(
        &self,
        plan: &AdapterDispatchPlanProjection,
    ) -> ServerResult<Option<String>> {
        Ok(self
            .controller
            .state()
            .recent_events_for_session(&plan.session_id, 200)
            .map_err(ServerError::State)?
            .into_iter()
            .find(|event| {
                event.kind == "adapter.dispatch_planned"
                    && event.payload_json.contains(&plan.dispatch_plan_id)
            })
            .and_then(|event| event.turn_id))
    }

    pub(crate) fn append_dispatch_gate(
        &self,
        origin: &ServerClientOrigin,
        plan: &AdapterDispatchPlanProjection,
        gate: &AdapterDispatchGateProjection,
    ) -> ServerResult<i64> {
        let event = NewEvent {
            event_id: format!(
                "event-server-dispatch-gate-{}",
                stable_hash(gate.dispatch_gate_id.as_bytes())
            ),
            kind: EventKind::AdapterDispatchGateChecked,
            actor: origin.actor_id.clone(),
            project_id: Some(gate.project_id.clone()),
            task_id: None,
            agent_id: Some(plan.agent_id.clone()),
            session_id: Some(plan.session_id.clone()),
            run_id: Some(plan.run_id.clone()),
            turn_id: None,
            item_id: Some(gate.dispatch_gate_id.clone()),
            payload_json: serde_json::json!({
                "dispatch_plan_id": gate.dispatch_plan_id,
                "provider_cli_execution_allowed": gate.provider_cli_execution_allowed,
                "provider_cli_executed": false,
                "reason_codes": gate.reason_codes,
            })
            .to_string(),
            idempotency_key: Some(format!(
                "server-dispatch-gate:{}:{}",
                gate.dispatch_plan_id, gate.reason_codes
            )),
            redaction_state: RedactionState::Safe,
        };
        self.controller
            .state()
            .append_event(
                event,
                &[ProjectionRecord::AdapterDispatchGate(gate.clone())],
            )
            .map_err(ServerError::State)
    }

    pub(crate) fn append_prompt_materialization(
        &self,
        origin: &ServerClientOrigin,
        plan: &AdapterDispatchPlanProjection,
        materialization: &AdapterDispatchPromptMaterializationProjection,
    ) -> ServerResult<i64> {
        let event = NewEvent {
            event_id: format!(
                "event-server-dispatch-prompt-materialization-{}",
                stable_hash(materialization.materialization_id.as_bytes())
            ),
            kind: EventKind::AdapterDispatchPromptMaterialized,
            actor: origin.actor_id.clone(),
            project_id: Some(materialization.project_id.clone()),
            task_id: None,
            agent_id: Some(plan.agent_id.clone()),
            session_id: Some(plan.session_id.clone()),
            run_id: Some(plan.run_id.clone()),
            turn_id: None,
            item_id: Some(materialization.materialization_id.clone()),
            payload_json: serde_json::json!({
                "dispatch_plan_id": materialization.dispatch_plan_id,
                "prompt_source_id": materialization.prompt_source_id,
                "status": materialization.status,
                "raw_prompt_policy": materialization.raw_prompt_policy,
            })
            .to_string(),
            idempotency_key: Some(format!(
                "server-dispatch-prompt-materialization:{}:{}",
                materialization.dispatch_plan_id, materialization.materialization_id
            )),
            redaction_state: RedactionState::Safe,
        };
        self.controller
            .state()
            .append_event(
                event,
                &[ProjectionRecord::AdapterDispatchPromptMaterialization(
                    materialization.clone(),
                )],
            )
            .map_err(ServerError::State)
    }

    pub(crate) fn append_execution_request(
        &self,
        origin: &ServerClientOrigin,
        plan: &AdapterDispatchPlanProjection,
        request: &capo_state::AdapterDispatchExecutionRequestProjection,
    ) -> ServerResult<i64> {
        let event = NewEvent {
            event_id: format!(
                "event-server-dispatch-execution-request-{}",
                stable_hash(request.execution_request_id.as_bytes())
            ),
            kind: EventKind::AdapterDispatchExecutionRequested,
            actor: origin.actor_id.clone(),
            project_id: Some(request.project_id.clone()),
            task_id: None,
            agent_id: Some(plan.agent_id.clone()),
            session_id: Some(plan.session_id.clone()),
            run_id: Some(plan.run_id.clone()),
            turn_id: None,
            item_id: Some(request.execution_request_id.clone()),
            payload_json: serde_json::json!({
                "dispatch_plan_id": request.dispatch_plan_id,
                "dispatch_gate_id": request.dispatch_gate_id,
                "provider_cli_execution_allowed": request.provider_cli_execution_allowed,
                "provider_cli_executed": false,
                "status": request.status,
            })
            .to_string(),
            idempotency_key: Some(format!(
                "server-dispatch-execution-request:{}:{}",
                request.dispatch_plan_id, request.execution_request_id
            )),
            redaction_state: RedactionState::Safe,
        };
        self.controller
            .state()
            .append_event(
                event,
                &[ProjectionRecord::AdapterDispatchExecutionRequest(
                    request.clone(),
                )],
            )
            .map_err(ServerError::State)
    }

    pub(crate) fn dispatch_execution_projection(
        &self,
        plan: &AdapterDispatchPlanProjection,
        request: &capo_state::AdapterDispatchExecutionRequestProjection,
        outcome: DispatchExecutionOutcome<'_>,
    ) -> AdapterDispatchExecutionProjection {
        AdapterDispatchExecutionProjection {
            dispatch_execution_id: format!(
                "server-dispatch-execution-{}-{}",
                stable_hash(plan.dispatch_plan_id.as_bytes()),
                stable_hash(
                    format!(
                        "{}:{}:{}",
                        outcome.status, outcome.provider_cli_executed, outcome.reason_codes
                    )
                    .as_bytes()
                )
            ),
            project_id: plan.project_id.clone(),
            dispatch_plan_id: plan.dispatch_plan_id.clone(),
            execution_request_id: request.execution_request_id.clone(),
            adapter_kind: plan.adapter_kind.clone(),
            session_id: plan.session_id.clone(),
            run_id: plan.run_id.clone(),
            provider_cli_execution_allowed: request.provider_cli_execution_allowed,
            provider_cli_executed: outcome.provider_cli_executed,
            status: outcome.status.to_string(),
            exit_code: outcome.exit_code,
            runtime_process_ref: outcome.runtime_process_ref,
            stdout_artifact_id: outcome.stdout_artifact_id,
            stderr_artifact_id: outcome.stderr_artifact_id,
            artifact_root: plan.artifact_root.clone(),
            credential_scan_status: outcome.credential_scan_status.to_string(),
            raw_prompt_policy: "not_rendered".to_string(),
            raw_output_policy: outcome.raw_output_policy.to_string(),
            reason_codes: outcome.reason_codes.to_string(),
            updated_sequence: 0,
        }
    }

    pub(crate) fn append_dispatch_execution(
        &self,
        origin: &ServerClientOrigin,
        plan: &AdapterDispatchPlanProjection,
        execution: &AdapterDispatchExecutionProjection,
    ) -> ServerResult<i64> {
        let event = NewEvent {
            event_id: format!(
                "event-server-dispatch-execution-{}",
                stable_hash(execution.dispatch_execution_id.as_bytes())
            ),
            kind: EventKind::AdapterDispatchExecuted,
            actor: origin.actor_id.clone(),
            project_id: Some(execution.project_id.clone()),
            task_id: None,
            agent_id: Some(plan.agent_id.clone()),
            session_id: Some(execution.session_id.clone()),
            run_id: Some(execution.run_id.clone()),
            turn_id: None,
            item_id: Some(execution.dispatch_execution_id.clone()),
            payload_json: serde_json::json!({
                "dispatch_plan_id": execution.dispatch_plan_id,
                "execution_request_id": execution.execution_request_id,
                "provider_cli_execution_allowed": execution.provider_cli_execution_allowed,
                "provider_cli_executed": execution.provider_cli_executed,
                "status": execution.status,
                "runtime_process_ref": execution.runtime_process_ref,
                "credential_scan_status": execution.credential_scan_status,
                "raw_prompt_policy": execution.raw_prompt_policy,
                "raw_output_policy": execution.raw_output_policy,
            })
            .to_string(),
            idempotency_key: Some(format!(
                "server-dispatch-execution:{}:{}",
                execution.dispatch_plan_id, execution.dispatch_execution_id
            )),
            redaction_state: RedactionState::Safe,
        };
        self.controller
            .state()
            .append_event(
                event,
                &[ProjectionRecord::AdapterDispatchExecution(
                    execution.clone(),
                )],
            )
            .map_err(ServerError::State)
    }

    pub(crate) fn append_dispatch_run_exit(
        &self,
        origin: &ServerClientOrigin,
        plan: &AdapterDispatchPlanProjection,
        run: &RunProjection,
    ) -> ServerResult<i64> {
        self.append_dispatch_run_exit_with_metadata(
            origin,
            plan,
            run,
            false,
            "deterministic_fixture_ingested_without_provider_cli",
        )
    }

    pub(crate) fn append_dispatch_run_exit_with_metadata(
        &self,
        origin: &ServerClientOrigin,
        plan: &AdapterDispatchPlanProjection,
        run: &RunProjection,
        provider_cli_executed: bool,
        reason: &str,
    ) -> ServerResult<i64> {
        let completed_run = RunProjection {
            run_id: run.run_id.clone(),
            session_id: run.session_id.clone(),
            status: "exited".to_string(),
            recovery_of_run_id: run.recovery_of_run_id.clone(),
            updated_sequence: 0,
        };
        let event = NewEvent {
            event_id: format!(
                "event-server-dispatch-run-exit-{}-{}",
                stable_hash(plan.dispatch_plan_id.as_bytes()),
                stable_hash(format!("{provider_cli_executed}:{reason}").as_bytes())
            ),
            kind: EventKind::RunExited,
            actor: origin.actor_id.clone(),
            project_id: Some(plan.project_id.clone()),
            task_id: None,
            agent_id: Some(plan.agent_id.clone()),
            session_id: Some(plan.session_id.clone()),
            run_id: Some(plan.run_id.clone()),
            turn_id: None,
            item_id: Some(plan.dispatch_plan_id.clone()),
            payload_json: serde_json::json!({
                "dispatch_plan_id": plan.dispatch_plan_id,
                "status": "exited",
                "provider_cli_executed": provider_cli_executed,
                "reason": reason,
            })
            .to_string(),
            idempotency_key: Some(format!(
                "server-dispatch-run-exit:{}:{}:{}",
                plan.dispatch_plan_id, provider_cli_executed, reason
            )),
            redaction_state: RedactionState::Safe,
        };
        self.controller
            .state()
            .append_event(event, &[ProjectionRecord::Run(completed_run)])
            .map_err(ServerError::State)
    }

    pub(crate) fn dispatch_replay_projection(
        &self,
        plan: &AdapterDispatchPlanProjection,
        gate: &AdapterDispatchGateProjection,
        fixture_name: &str,
        fixture_hash: &str,
        report: &capo_controller::AdapterReplayReport,
    ) -> capo_state::AdapterDispatchReplayProjection {
        self.dispatch_replay_projection_with_metadata(
            plan,
            gate,
            fixture_name,
            fixture_hash,
            report,
            DispatchReplayMetadata {
                provider_cli_executed: false,
                raw_content_policy: "content_hashed_not_rendered",
            },
        )
    }

    pub(crate) fn dispatch_replay_projection_with_metadata(
        &self,
        plan: &AdapterDispatchPlanProjection,
        gate: &AdapterDispatchGateProjection,
        fixture_name: &str,
        fixture_hash: &str,
        report: &capo_controller::AdapterReplayReport,
        metadata: DispatchReplayMetadata<'_>,
    ) -> capo_state::AdapterDispatchReplayProjection {
        capo_state::AdapterDispatchReplayProjection {
            dispatch_replay_id: format!(
                "server-dispatch-replay-{}-{}",
                stable_hash(plan.dispatch_plan_id.as_bytes()),
                stable_hash(fixture_hash.as_bytes())
            ),
            project_id: plan.project_id.clone(),
            dispatch_plan_id: plan.dispatch_plan_id.clone(),
            dispatch_gate_id: gate.dispatch_gate_id.clone(),
            adapter_kind: plan.adapter_kind.clone(),
            session_id: plan.session_id.clone(),
            run_id: plan.run_id.clone(),
            fixture_path: fixture_name.to_string(),
            fixture_hash: fixture_hash.to_string(),
            input_event_count: report.input_event_count as i64,
            appended_event_count: report.appended_event_count as i64,
            tool_event_count: report.tool_event_count as i64,
            summary_event_count: report.summary_event_count as i64,
            completed_turn_count: report.completed_turn_count as i64,
            provider_cli_executed: metadata.provider_cli_executed,
            raw_content_policy: metadata.raw_content_policy.to_string(),
            updated_sequence: 0,
        }
    }

    pub(crate) fn append_dispatch_replay(
        &self,
        origin: &ServerClientOrigin,
        plan: &AdapterDispatchPlanProjection,
        replay: &capo_state::AdapterDispatchReplayProjection,
    ) -> ServerResult<i64> {
        let event = NewEvent {
            event_id: format!(
                "event-server-dispatch-replay-{}",
                stable_hash(replay.dispatch_replay_id.as_bytes())
            ),
            kind: EventKind::AdapterDispatchReplayed,
            actor: origin.actor_id.clone(),
            project_id: Some(replay.project_id.clone()),
            task_id: None,
            agent_id: Some(plan.agent_id.clone()),
            session_id: Some(replay.session_id.clone()),
            run_id: Some(replay.run_id.clone()),
            turn_id: None,
            item_id: Some(replay.dispatch_replay_id.clone()),
            payload_json: serde_json::json!({
                "dispatch_plan_id": replay.dispatch_plan_id,
                "dispatch_gate_id": replay.dispatch_gate_id,
                "fixture_hash": replay.fixture_hash,
                "provider_cli_executed": replay.provider_cli_executed,
                "raw_content_policy": replay.raw_content_policy,
            })
            .to_string(),
            idempotency_key: Some(format!(
                "server-dispatch-replay:{}:{}",
                replay.dispatch_plan_id, replay.fixture_hash
            )),
            redaction_state: RedactionState::Safe,
        };
        self.controller
            .state()
            .append_event(
                event,
                &[ProjectionRecord::AdapterDispatchReplay(replay.clone())],
            )
            .map_err(ServerError::State)
    }
}
