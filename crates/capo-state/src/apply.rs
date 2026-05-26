use capo_core::{AgentId, EvidenceId, RunId, SessionId, TaskId, ToolCallId};
use rusqlite::{Transaction, params};

use crate::codec::validate_projection_json;
use crate::{ProjectionRecord, StateResult};

pub(crate) fn update_watermark(
    transaction: &Transaction<'_>,
    name: &str,
    sequence: i64,
) -> StateResult<()> {
    transaction.execute(
        "INSERT INTO projection_watermarks(name, last_sequence)
         VALUES (?1, ?2)
         ON CONFLICT(name) DO UPDATE SET last_sequence = excluded.last_sequence",
        params![name, sequence],
    )?;
    Ok(())
}

pub(crate) fn apply_projection_record(
    transaction: &Transaction<'_>,
    sequence: i64,
    record: &ProjectionRecord,
) -> StateResult<()> {
    match record {
        ProjectionRecord::Project(project) => transaction.execute(
            "INSERT INTO projects(project_id, name, status, updated_sequence)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(project_id) DO UPDATE SET
                name = excluded.name,
                status = excluded.status,
                updated_sequence = excluded.updated_sequence",
            params![
                project.project_id.as_str(),
                project.name,
                project.status,
                sequence
            ],
        )?,
        ProjectionRecord::Task(task) => transaction.execute(
            "INSERT INTO tasks(
                task_id, project_id, title, capo_execution_status, active_session_id,
                latest_summary, evidence_id, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(task_id) DO UPDATE SET
                project_id = excluded.project_id,
                title = excluded.title,
                capo_execution_status = excluded.capo_execution_status,
                active_session_id = excluded.active_session_id,
                latest_summary = excluded.latest_summary,
                evidence_id = excluded.evidence_id,
                updated_sequence = excluded.updated_sequence",
            params![
                task.task_id.as_str(),
                task.project_id.as_str(),
                task.title,
                task.capo_execution_status,
                task.active_session_id.as_ref().map(SessionId::as_str),
                task.latest_summary,
                task.evidence_id.as_ref().map(EvidenceId::as_str),
                sequence,
            ],
        )?,
        ProjectionRecord::Agent(agent) => transaction.execute(
            "INSERT INTO agents(agent_id, project_id, name, status, current_session_id, updated_sequence)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(agent_id) DO UPDATE SET
                project_id = excluded.project_id,
                name = excluded.name,
                status = excluded.status,
                current_session_id = excluded.current_session_id,
                updated_sequence = excluded.updated_sequence",
            params![
                agent.agent_id.as_str(),
                agent.project_id.as_str(),
                agent.name,
                agent.status,
                agent.current_session_id.as_ref().map(SessionId::as_str),
                sequence,
            ],
        )?,
        ProjectionRecord::Session(session) => transaction.execute(
            "INSERT INTO sessions(
                session_id, project_id, task_id, agent_id, title, status, current_goal,
                latest_summary, latest_confidence, latest_blocker, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(session_id) DO UPDATE SET
                project_id = excluded.project_id,
                task_id = excluded.task_id,
                agent_id = excluded.agent_id,
                title = excluded.title,
                status = excluded.status,
                current_goal = excluded.current_goal,
                latest_summary = excluded.latest_summary,
                latest_confidence = excluded.latest_confidence,
                latest_blocker = excluded.latest_blocker,
                updated_sequence = excluded.updated_sequence",
            params![
                session.session_id.as_str(),
                session.project_id.as_str(),
                session.task_id.as_ref().map(TaskId::as_str),
                session.agent_id.as_str(),
                session.title,
                session.status,
                session.current_goal,
                session.latest_summary,
                session.latest_confidence,
                session.latest_blocker,
                sequence,
            ],
        )?,
        ProjectionRecord::Run(run) => transaction.execute(
            "INSERT INTO runs(run_id, session_id, status, recovery_of_run_id, updated_sequence)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(run_id) DO UPDATE SET
                session_id = excluded.session_id,
                status = excluded.status,
                recovery_of_run_id = excluded.recovery_of_run_id,
                updated_sequence = excluded.updated_sequence",
            params![
                run.run_id.as_str(),
                run.session_id.as_str(),
                run.status,
                run.recovery_of_run_id.as_ref().map(RunId::as_str),
                sequence,
            ],
        )?,
        ProjectionRecord::CapabilityGrant(grant) => {
            validate_projection_json(
                "capability_grant",
                &grant.capability_grant_id,
                "scope_json",
                &grant.scope_json,
            )?;
            validate_projection_json(
                "capability_grant",
                &grant.capability_grant_id,
                "subject_json",
                &grant.subject_json,
            )?;
            transaction.execute(
                "INSERT INTO capability_grants(
                capability_grant_id, capability_profile_id, scope_json, effect,
                subject_json, decision_source, persistence, explanation, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(capability_grant_id) DO UPDATE SET
                capability_profile_id = excluded.capability_profile_id,
                scope_json = excluded.scope_json,
                effect = excluded.effect,
                subject_json = excluded.subject_json,
                decision_source = excluded.decision_source,
                persistence = excluded.persistence,
                explanation = excluded.explanation,
                updated_sequence = excluded.updated_sequence",
            params![
                grant.capability_grant_id,
                grant.capability_profile_id,
                grant.scope_json,
                grant.effect,
                grant.subject_json,
                grant.decision_source,
                grant.persistence,
                grant.explanation,
                sequence,
            ],
            )?
        }
        ProjectionRecord::PermissionApproval(approval) => {
            validate_projection_json(
                "permission_approval",
                &approval.approval_id,
                "scope_json",
                &approval.scope_json,
            )?;
            validate_projection_json(
                "permission_approval",
                &approval.approval_id,
                "subject_json",
                &approval.subject_json,
            )?;
            transaction.execute(
                "INSERT INTO permission_approvals(
                approval_id, project_id, session_id, tool_call_id, capability_profile_id,
                scope_json, subject_json, status, requested_by, reason, decision,
                capability_grant_id, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(approval_id) DO UPDATE SET
                project_id = excluded.project_id,
                session_id = excluded.session_id,
                tool_call_id = excluded.tool_call_id,
                capability_profile_id = excluded.capability_profile_id,
                scope_json = excluded.scope_json,
                subject_json = excluded.subject_json,
                status = excluded.status,
                requested_by = excluded.requested_by,
                reason = excluded.reason,
                decision = excluded.decision,
                capability_grant_id = excluded.capability_grant_id,
                updated_sequence = excluded.updated_sequence",
            params![
                approval.approval_id,
                approval.project_id.as_str(),
                approval.session_id.as_ref().map(SessionId::as_str),
                approval.tool_call_id.as_ref().map(ToolCallId::as_str),
                approval.capability_profile_id,
                approval.scope_json,
                approval.subject_json,
                approval.status,
                approval.requested_by,
                approval.reason,
                approval.decision,
                approval.capability_grant_id,
                sequence,
            ],
            )?
        }
        ProjectionRecord::ConnectivityExposure(exposure) => transaction.execute(
            "INSERT INTO connectivity_exposures(
                exposure_id, project_id, connectivity_endpoint_id, owner_kind, owner_id,
                channel_kind, exposure, permission_scope, status, capability_grant_id,
                health_status, reachable, revoked_at, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(exposure_id) DO UPDATE SET
                project_id = excluded.project_id,
                connectivity_endpoint_id = excluded.connectivity_endpoint_id,
                owner_kind = excluded.owner_kind,
                owner_id = excluded.owner_id,
                channel_kind = excluded.channel_kind,
                exposure = excluded.exposure,
                permission_scope = excluded.permission_scope,
                status = excluded.status,
                capability_grant_id = excluded.capability_grant_id,
                health_status = excluded.health_status,
                reachable = excluded.reachable,
                revoked_at = excluded.revoked_at,
                updated_sequence = excluded.updated_sequence",
            params![
                exposure.exposure_id,
                exposure.project_id.as_str(),
                exposure.connectivity_endpoint_id,
                exposure.owner_kind,
                exposure.owner_id,
                exposure.channel_kind,
                exposure.exposure,
                exposure.permission_scope,
                exposure.status,
                exposure.capability_grant_id,
                exposure.health_status,
                if exposure.reachable { 1 } else { 0 },
                exposure.revoked_at,
                sequence,
            ],
        )?,
        ProjectionRecord::RuntimeTarget(target) => transaction.execute(
            "INSERT INTO runtime_targets(
                runtime_target_id, project_id, name, runner_kind, workspace_root,
                artifact_root, default_cwd, capability_profile_id, connectivity_endpoint_id,
                status, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(runtime_target_id) DO UPDATE SET
                project_id = excluded.project_id,
                name = excluded.name,
                runner_kind = excluded.runner_kind,
                workspace_root = excluded.workspace_root,
                artifact_root = excluded.artifact_root,
                default_cwd = excluded.default_cwd,
                capability_profile_id = excluded.capability_profile_id,
                connectivity_endpoint_id = excluded.connectivity_endpoint_id,
                status = excluded.status,
                updated_sequence = excluded.updated_sequence",
            params![
                target.runtime_target_id,
                target.project_id.as_str(),
                target.name,
                target.runner_kind,
                target.workspace_root,
                target.artifact_root,
                target.default_cwd,
                target.capability_profile_id,
                target.connectivity_endpoint_id,
                target.status,
                sequence,
            ],
        )?,
        ProjectionRecord::AdapterReadiness(readiness) => transaction.execute(
            "INSERT INTO adapter_readiness(
                adapter_kind, project_id, program, opt_in_env, opted_in, smoke_status,
                credential_policy, expected_marker, env_allowlist_count, redaction_rule_count,
                output_limit_bytes, dogfood_blocker, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(adapter_kind, project_id) DO UPDATE SET
                program = excluded.program,
                opt_in_env = excluded.opt_in_env,
                opted_in = excluded.opted_in,
                smoke_status = excluded.smoke_status,
                credential_policy = excluded.credential_policy,
                expected_marker = excluded.expected_marker,
                env_allowlist_count = excluded.env_allowlist_count,
                redaction_rule_count = excluded.redaction_rule_count,
                output_limit_bytes = excluded.output_limit_bytes,
                dogfood_blocker = excluded.dogfood_blocker,
                updated_sequence = excluded.updated_sequence",
            params![
                readiness.adapter_kind,
                readiness.project_id.as_str(),
                readiness.program,
                readiness.opt_in_env,
                if readiness.opted_in { 1 } else { 0 },
                readiness.smoke_status,
                readiness.credential_policy,
                readiness.expected_marker,
                readiness.env_allowlist_count,
                readiness.redaction_rule_count,
                readiness.output_limit_bytes,
                readiness.dogfood_blocker,
                sequence,
            ],
        )?,
        ProjectionRecord::AdapterSmokeReport(report) => transaction.execute(
            "INSERT INTO adapter_smoke_reports(
                smoke_report_id, project_id, adapter_kind, smoke_status,
                credential_scan_status, marker_found, artifact_root, reason,
                dogfood_readiness_effect, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(smoke_report_id) DO UPDATE SET
                project_id = excluded.project_id,
                adapter_kind = excluded.adapter_kind,
                smoke_status = excluded.smoke_status,
                credential_scan_status = excluded.credential_scan_status,
                marker_found = excluded.marker_found,
                artifact_root = excluded.artifact_root,
                reason = excluded.reason,
                dogfood_readiness_effect = excluded.dogfood_readiness_effect,
                updated_sequence = excluded.updated_sequence",
            params![
                report.smoke_report_id,
                report.project_id.as_str(),
                report.adapter_kind,
                report.smoke_status,
                report.credential_scan_status,
                if report.marker_found { 1 } else { 0 },
                report.artifact_root,
                report.reason,
                report.dogfood_readiness_effect,
                sequence,
            ],
        )?,
        ProjectionRecord::AdapterDispatchPlan(plan) => transaction.execute(
            "INSERT INTO adapter_dispatch_plans(
                dispatch_plan_id, project_id, adapter_kind, provider_kind,
                credential_scope, agent_id, agent_name, session_id, run_id,
                runtime_program, runtime_arg_count, runtime_prompt_policy, runtime_cwd,
                artifact_root, request_env_count, env_allowlist_count, redaction_rule_count,
                stdout_format, stderr_policy, provider_cli_executed, status, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)
             ON CONFLICT(dispatch_plan_id) DO UPDATE SET
                project_id = excluded.project_id,
                adapter_kind = excluded.adapter_kind,
                provider_kind = excluded.provider_kind,
                credential_scope = excluded.credential_scope,
                agent_id = excluded.agent_id,
                agent_name = excluded.agent_name,
                session_id = excluded.session_id,
                run_id = excluded.run_id,
                runtime_program = excluded.runtime_program,
                runtime_arg_count = excluded.runtime_arg_count,
                runtime_prompt_policy = excluded.runtime_prompt_policy,
                runtime_cwd = excluded.runtime_cwd,
                artifact_root = excluded.artifact_root,
                request_env_count = excluded.request_env_count,
                env_allowlist_count = excluded.env_allowlist_count,
                redaction_rule_count = excluded.redaction_rule_count,
                stdout_format = excluded.stdout_format,
                stderr_policy = excluded.stderr_policy,
                provider_cli_executed = excluded.provider_cli_executed,
                status = excluded.status,
                updated_sequence = excluded.updated_sequence",
            params![
                plan.dispatch_plan_id,
                plan.project_id.as_str(),
                plan.adapter_kind,
                plan.provider_kind,
                plan.credential_scope,
                plan.agent_id.as_str(),
                plan.agent_name,
                plan.session_id.as_str(),
                plan.run_id.as_str(),
                plan.runtime_program,
                plan.runtime_arg_count,
                plan.runtime_prompt_policy,
                plan.runtime_cwd,
                plan.artifact_root,
                plan.request_env_count,
                plan.env_allowlist_count,
                plan.redaction_rule_count,
                plan.stdout_format,
                plan.stderr_policy,
                if plan.provider_cli_executed { 1 } else { 0 },
                plan.status,
                sequence,
            ],
        )?,
        ProjectionRecord::AdapterDispatchGate(gate) => transaction.execute(
            "INSERT INTO adapter_dispatch_gates(
                dispatch_gate_id, project_id, dispatch_plan_id, adapter_kind,
                provider_cli_execution_allowed, status, required_dogfood_gate,
                reason_codes, provider_cli_executed, runtime_prompt_policy, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(dispatch_gate_id) DO UPDATE SET
                project_id = excluded.project_id,
                dispatch_plan_id = excluded.dispatch_plan_id,
                adapter_kind = excluded.adapter_kind,
                provider_cli_execution_allowed = excluded.provider_cli_execution_allowed,
                status = excluded.status,
                required_dogfood_gate = excluded.required_dogfood_gate,
                reason_codes = excluded.reason_codes,
                provider_cli_executed = excluded.provider_cli_executed,
                runtime_prompt_policy = excluded.runtime_prompt_policy,
                updated_sequence = excluded.updated_sequence",
            params![
                gate.dispatch_gate_id,
                gate.project_id.as_str(),
                gate.dispatch_plan_id,
                gate.adapter_kind,
                if gate.provider_cli_execution_allowed { 1 } else { 0 },
                gate.status,
                gate.required_dogfood_gate,
                gate.reason_codes,
                if gate.provider_cli_executed { 1 } else { 0 },
                gate.runtime_prompt_policy,
                sequence,
            ],
        )?,
        ProjectionRecord::AdapterDispatchReplay(replay) => transaction.execute(
            "INSERT INTO adapter_dispatch_replays(
                dispatch_replay_id, project_id, dispatch_plan_id, dispatch_gate_id,
                adapter_kind, session_id, run_id, fixture_path, fixture_hash,
                input_event_count, appended_event_count, tool_event_count,
                summary_event_count, completed_turn_count, provider_cli_executed,
                raw_content_policy, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
             ON CONFLICT(dispatch_replay_id) DO UPDATE SET
                project_id = excluded.project_id,
                dispatch_plan_id = excluded.dispatch_plan_id,
                dispatch_gate_id = excluded.dispatch_gate_id,
                adapter_kind = excluded.adapter_kind,
                session_id = excluded.session_id,
                run_id = excluded.run_id,
                fixture_path = excluded.fixture_path,
                fixture_hash = excluded.fixture_hash,
                input_event_count = excluded.input_event_count,
                appended_event_count = excluded.appended_event_count,
                tool_event_count = excluded.tool_event_count,
                summary_event_count = excluded.summary_event_count,
                completed_turn_count = excluded.completed_turn_count,
                provider_cli_executed = excluded.provider_cli_executed,
                raw_content_policy = excluded.raw_content_policy,
                updated_sequence = excluded.updated_sequence",
            params![
                replay.dispatch_replay_id,
                replay.project_id.as_str(),
                replay.dispatch_plan_id,
                replay.dispatch_gate_id,
                replay.adapter_kind,
                replay.session_id.as_str(),
                replay.run_id.as_str(),
                replay.fixture_path,
                replay.fixture_hash,
                replay.input_event_count,
                replay.appended_event_count,
                replay.tool_event_count,
                replay.summary_event_count,
                replay.completed_turn_count,
                if replay.provider_cli_executed { 1 } else { 0 },
                replay.raw_content_policy,
                sequence,
            ],
        )?,
        ProjectionRecord::AdapterDispatchExecutionRequest(request) => transaction.execute(
            "INSERT INTO adapter_dispatch_execution_requests(
                execution_request_id, project_id, dispatch_plan_id, dispatch_gate_id,
                adapter_kind, provider_cli_execution_allowed, provider_cli_executed,
                status, opt_in_env, runtime_prompt_policy, reason_codes, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(execution_request_id) DO UPDATE SET
                project_id = excluded.project_id,
                dispatch_plan_id = excluded.dispatch_plan_id,
                dispatch_gate_id = excluded.dispatch_gate_id,
                adapter_kind = excluded.adapter_kind,
                provider_cli_execution_allowed = excluded.provider_cli_execution_allowed,
                provider_cli_executed = excluded.provider_cli_executed,
                status = excluded.status,
                opt_in_env = excluded.opt_in_env,
                runtime_prompt_policy = excluded.runtime_prompt_policy,
                reason_codes = excluded.reason_codes,
                updated_sequence = excluded.updated_sequence",
            params![
                request.execution_request_id,
                request.project_id.as_str(),
                request.dispatch_plan_id,
                request.dispatch_gate_id,
                request.adapter_kind,
                if request.provider_cli_execution_allowed { 1 } else { 0 },
                if request.provider_cli_executed { 1 } else { 0 },
                request.status,
                request.opt_in_env,
                request.runtime_prompt_policy,
                request.reason_codes,
                sequence,
            ],
        )?,
        ProjectionRecord::AdapterDispatchExecution(execution) => transaction.execute(
            "INSERT INTO adapter_dispatch_executions(
                dispatch_execution_id, project_id, dispatch_plan_id,
                execution_request_id, adapter_kind, session_id, run_id,
                provider_cli_execution_allowed, provider_cli_executed, status,
                exit_code, runtime_process_ref, stdout_artifact_id, stderr_artifact_id,
                artifact_root, credential_scan_status, raw_prompt_policy,
                raw_output_policy, reason_codes, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
             ON CONFLICT(dispatch_execution_id) DO UPDATE SET
                project_id = excluded.project_id,
                dispatch_plan_id = excluded.dispatch_plan_id,
                execution_request_id = excluded.execution_request_id,
                adapter_kind = excluded.adapter_kind,
                session_id = excluded.session_id,
                run_id = excluded.run_id,
                provider_cli_execution_allowed = excluded.provider_cli_execution_allowed,
                provider_cli_executed = excluded.provider_cli_executed,
                status = excluded.status,
                exit_code = excluded.exit_code,
                runtime_process_ref = excluded.runtime_process_ref,
                stdout_artifact_id = excluded.stdout_artifact_id,
                stderr_artifact_id = excluded.stderr_artifact_id,
                artifact_root = excluded.artifact_root,
                credential_scan_status = excluded.credential_scan_status,
                raw_prompt_policy = excluded.raw_prompt_policy,
                raw_output_policy = excluded.raw_output_policy,
                reason_codes = excluded.reason_codes,
                updated_sequence = excluded.updated_sequence",
            params![
                execution.dispatch_execution_id,
                execution.project_id.as_str(),
                execution.dispatch_plan_id,
                execution.execution_request_id,
                execution.adapter_kind,
                execution.session_id.as_str(),
                execution.run_id.as_str(),
                if execution.provider_cli_execution_allowed { 1 } else { 0 },
                if execution.provider_cli_executed { 1 } else { 0 },
                execution.status,
                execution.exit_code,
                execution.runtime_process_ref,
                execution.stdout_artifact_id,
                execution.stderr_artifact_id,
                execution.artifact_root,
                execution.credential_scan_status,
                execution.raw_prompt_policy,
                execution.raw_output_policy,
                execution.reason_codes,
                sequence,
            ],
        )?,
        ProjectionRecord::AdapterDispatchPromptSource(source) => transaction.execute(
            "INSERT INTO adapter_dispatch_prompt_sources(
                prompt_source_id, project_id, dispatch_plan_id, prompt_hash,
                source_kind, source_ref, source_hash, materialization_status,
                raw_prompt_policy, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(prompt_source_id) DO UPDATE SET
                project_id = excluded.project_id,
                dispatch_plan_id = excluded.dispatch_plan_id,
                prompt_hash = excluded.prompt_hash,
                source_kind = excluded.source_kind,
                source_ref = excluded.source_ref,
                source_hash = excluded.source_hash,
                materialization_status = excluded.materialization_status,
                raw_prompt_policy = excluded.raw_prompt_policy,
                updated_sequence = excluded.updated_sequence",
            params![
                source.prompt_source_id,
                source.project_id.as_str(),
                source.dispatch_plan_id,
                source.prompt_hash,
                source.source_kind,
                source.source_ref,
                source.source_hash,
                source.materialization_status,
                source.raw_prompt_policy,
                sequence,
            ],
        )?,
        ProjectionRecord::AdapterDispatchPromptMaterialization(materialization) => {
            transaction.execute(
                "INSERT INTO adapter_dispatch_prompt_materializations(
                    materialization_id, project_id, dispatch_plan_id, prompt_source_id,
                    source_kind, source_ref, expected_source_hash, observed_source_hash,
                    expected_prompt_hash, materialized_prompt_hash, status,
                    raw_prompt_policy, reason_codes, updated_sequence
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(materialization_id) DO UPDATE SET
                    project_id = excluded.project_id,
                    dispatch_plan_id = excluded.dispatch_plan_id,
                    prompt_source_id = excluded.prompt_source_id,
                    source_kind = excluded.source_kind,
                    source_ref = excluded.source_ref,
                    expected_source_hash = excluded.expected_source_hash,
                    observed_source_hash = excluded.observed_source_hash,
                    expected_prompt_hash = excluded.expected_prompt_hash,
                    materialized_prompt_hash = excluded.materialized_prompt_hash,
                    status = excluded.status,
                    raw_prompt_policy = excluded.raw_prompt_policy,
                    reason_codes = excluded.reason_codes,
                    updated_sequence = excluded.updated_sequence",
                params![
                    materialization.materialization_id,
                    materialization.project_id.as_str(),
                    materialization.dispatch_plan_id,
                    materialization.prompt_source_id,
                    materialization.source_kind,
                    materialization.source_ref,
                    materialization.expected_source_hash,
                    materialization.observed_source_hash,
                    materialization.expected_prompt_hash,
                    materialization.materialized_prompt_hash,
                    materialization.status,
                    materialization.raw_prompt_policy,
                    materialization.reason_codes,
                    sequence,
                ],
            )
        }?,
        ProjectionRecord::ToolCall(tool_call) => transaction.execute(
            "INSERT INTO tool_calls(
                tool_call_id, session_id, turn_id, tool_name, tool_origin, status,
                input_artifact_id, output_artifact_id, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(tool_call_id) DO UPDATE SET
                session_id = excluded.session_id,
                turn_id = excluded.turn_id,
                tool_name = excluded.tool_name,
                tool_origin = excluded.tool_origin,
                status = excluded.status,
                input_artifact_id = excluded.input_artifact_id,
                output_artifact_id = excluded.output_artifact_id,
                updated_sequence = excluded.updated_sequence",
            params![
                tool_call.tool_call_id.as_str(),
                tool_call.session_id.as_str(),
                tool_call.turn_id,
                tool_call.tool_name,
                tool_call.tool_origin,
                tool_call.status,
                tool_call.input_artifact_id,
                tool_call.output_artifact_id,
                sequence,
            ],
        )?,
        ProjectionRecord::ToolObservation(observation) => transaction.execute(
            "INSERT INTO tool_observations(
                tool_observation_id, session_id, tool_call_id, source, external_tool_ref,
                tool_name, observed_status, instrumentation_level, confidence,
                raw_event_hash, artifact_id, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(tool_observation_id) DO UPDATE SET
                session_id = excluded.session_id,
                tool_call_id = excluded.tool_call_id,
                source = excluded.source,
                external_tool_ref = excluded.external_tool_ref,
                tool_name = excluded.tool_name,
                observed_status = excluded.observed_status,
                instrumentation_level = excluded.instrumentation_level,
                confidence = excluded.confidence,
                raw_event_hash = excluded.raw_event_hash,
                artifact_id = excluded.artifact_id,
                updated_sequence = excluded.updated_sequence",
            params![
                observation.tool_observation_id,
                observation.session_id.as_str(),
                observation.tool_call_id.as_ref().map(ToolCallId::as_str),
                observation.source,
                observation.external_tool_ref,
                observation.tool_name,
                observation.observed_status,
                observation.instrumentation_level,
                observation.confidence,
                observation.raw_event_hash,
                observation.artifact_id,
                sequence,
            ],
        )?,
        ProjectionRecord::MemoryPacketRef(packet) => transaction.execute(
            "INSERT INTO memory_packet_refs(
                memory_packet_id, project_id, task_id, agent_id, session_id, run_id,
                turn_id, packet_artifact_id, purpose, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(memory_packet_id) DO UPDATE SET
                project_id = excluded.project_id,
                task_id = excluded.task_id,
                agent_id = excluded.agent_id,
                session_id = excluded.session_id,
                run_id = excluded.run_id,
                turn_id = excluded.turn_id,
                packet_artifact_id = excluded.packet_artifact_id,
                purpose = excluded.purpose,
                updated_sequence = excluded.updated_sequence",
            params![
                packet.memory_packet_id.as_str(),
                packet.project_id.as_str(),
                packet.task_id.as_ref().map(TaskId::as_str),
                packet.agent_id.as_ref().map(AgentId::as_str),
                packet.session_id.as_ref().map(SessionId::as_str),
                packet.run_id.as_ref().map(RunId::as_str),
                packet.turn_id,
                packet.packet_artifact_id,
                packet.purpose,
                sequence,
            ],
        )?,
        ProjectionRecord::MemoryRecord(memory_record) => transaction.execute(
            "INSERT INTO memory_records(
                memory_record_id, project_id, scope, scope_owner_ref, subject_ref,
                sensitivity_classification, record_kind, subject, predicate, object, body,
                confidence, review_state, source_count, valid_from, valid_until,
                supersedes_memory_record_id, revoked_by_memory_record_id, redaction_state,
                invalidated_at, invalidation_reason, packet_item_ref, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)
             ON CONFLICT(memory_record_id) DO UPDATE SET
                project_id = excluded.project_id,
                scope = excluded.scope,
                scope_owner_ref = excluded.scope_owner_ref,
                subject_ref = excluded.subject_ref,
                sensitivity_classification = excluded.sensitivity_classification,
                record_kind = excluded.record_kind,
                subject = excluded.subject,
                predicate = excluded.predicate,
                object = excluded.object,
                body = excluded.body,
                confidence = excluded.confidence,
                review_state = excluded.review_state,
                source_count = excluded.source_count,
                valid_from = excluded.valid_from,
                valid_until = excluded.valid_until,
                supersedes_memory_record_id = excluded.supersedes_memory_record_id,
                revoked_by_memory_record_id = excluded.revoked_by_memory_record_id,
                redaction_state = excluded.redaction_state,
                invalidated_at = excluded.invalidated_at,
                invalidation_reason = excluded.invalidation_reason,
                packet_item_ref = excluded.packet_item_ref,
                updated_sequence = excluded.updated_sequence",
            params![
                memory_record.memory_record_id,
                memory_record.project_id.as_str(),
                memory_record.scope,
                memory_record.scope_owner_ref,
                memory_record.subject_ref,
                memory_record.sensitivity_classification,
                memory_record.record_kind,
                memory_record.subject,
                memory_record.predicate,
                memory_record.object,
                memory_record.body,
                memory_record.confidence,
                memory_record.review_state,
                memory_record.source_count,
                memory_record.valid_from,
                memory_record.valid_until,
                memory_record.supersedes_memory_record_id,
                memory_record.revoked_by_memory_record_id,
                memory_record.redaction_state,
                memory_record.invalidated_at,
                memory_record.invalidation_reason,
                memory_record.packet_item_ref,
                sequence,
            ],
        )?,
        ProjectionRecord::MemorySource(source) => transaction.execute(
            "INSERT INTO memory_sources(
                memory_source_id, memory_record_id, source_kind, source_event_id,
                source_artifact_id, source_path, source_anchor, source_content_hash,
                source_sequence, quote_artifact_id, observed_at, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(memory_source_id) DO UPDATE SET
                memory_record_id = excluded.memory_record_id,
                source_kind = excluded.source_kind,
                source_event_id = excluded.source_event_id,
                source_artifact_id = excluded.source_artifact_id,
                source_path = excluded.source_path,
                source_anchor = excluded.source_anchor,
                source_content_hash = excluded.source_content_hash,
                source_sequence = excluded.source_sequence,
                quote_artifact_id = excluded.quote_artifact_id,
                observed_at = excluded.observed_at,
                updated_sequence = excluded.updated_sequence",
            params![
                source.memory_source_id,
                source.memory_record_id,
                source.source_kind,
                source.source_event_id,
                source.source_artifact_id,
                source.source_path,
                source.source_anchor,
                source.source_content_hash,
                source.source_sequence,
                source.quote_artifact_id,
                source.observed_at,
                sequence,
            ],
        )?,
        ProjectionRecord::Evidence(evidence) => transaction.execute(
            "INSERT INTO evidence(
                evidence_id, project_id, task_id, session_id, run_id, kind,
                artifact_id, confidence, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(evidence_id) DO UPDATE SET
                project_id = excluded.project_id,
                task_id = excluded.task_id,
                session_id = excluded.session_id,
                run_id = excluded.run_id,
                kind = excluded.kind,
                artifact_id = excluded.artifact_id,
                confidence = excluded.confidence,
                updated_sequence = excluded.updated_sequence",
            params![
                evidence.evidence_id.as_str(),
                evidence.project_id.as_str(),
                evidence.task_id.as_ref().map(TaskId::as_str),
                evidence.session_id.as_ref().map(SessionId::as_str),
                evidence.run_id.as_ref().map(RunId::as_str),
                evidence.kind,
                evidence.artifact_id,
                evidence.confidence,
                sequence,
            ],
        )?,
        ProjectionRecord::TaskOutcomeReport(report) => transaction.execute(
            "INSERT INTO task_outcome_reports(
                task_outcome_report_id, project_id, task_id, session_id, run_id,
                outcome_status, started_sequence, completed_sequence, duration_sequence_span,
                action_count, tool_call_count, evidence_count, memory_packet_count,
                confidence, blocker, review_outcome, report_artifact_id, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18)
             ON CONFLICT(task_outcome_report_id) DO UPDATE SET
                project_id = excluded.project_id,
                task_id = excluded.task_id,
                session_id = excluded.session_id,
                run_id = excluded.run_id,
                outcome_status = excluded.outcome_status,
                started_sequence = excluded.started_sequence,
                completed_sequence = excluded.completed_sequence,
                duration_sequence_span = excluded.duration_sequence_span,
                action_count = excluded.action_count,
                tool_call_count = excluded.tool_call_count,
                evidence_count = excluded.evidence_count,
                memory_packet_count = excluded.memory_packet_count,
                confidence = excluded.confidence,
                blocker = excluded.blocker,
                review_outcome = excluded.review_outcome,
                report_artifact_id = excluded.report_artifact_id,
                updated_sequence = excluded.updated_sequence",
            params![
                report.task_outcome_report_id,
                report.project_id.as_str(),
                report.task_id.as_str(),
                report.session_id.as_str(),
                report.run_id.as_str(),
                report.outcome_status,
                report.started_sequence,
                report.completed_sequence,
                report.duration_sequence_span,
                report.action_count,
                report.tool_call_count,
                report.evidence_count,
                report.memory_packet_count,
                report.confidence,
                report.blocker,
                report.review_outcome,
                report.report_artifact_id,
                sequence,
            ],
        )?,
        ProjectionRecord::ReviewFinding(finding) => transaction.execute(
            "INSERT INTO review_findings(
                review_finding_id, project_id, task_id, session_id, run_id, tool_call_id,
                workpad_task_id, reviewer, finding_kind, severity, summary, status,
                evidence_artifact_id, follow_up, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(review_finding_id) DO UPDATE SET
                project_id = excluded.project_id,
                task_id = excluded.task_id,
                session_id = excluded.session_id,
                run_id = excluded.run_id,
                tool_call_id = excluded.tool_call_id,
                workpad_task_id = excluded.workpad_task_id,
                reviewer = excluded.reviewer,
                finding_kind = excluded.finding_kind,
                severity = excluded.severity,
                summary = excluded.summary,
                status = excluded.status,
                evidence_artifact_id = excluded.evidence_artifact_id,
                follow_up = excluded.follow_up,
                updated_sequence = excluded.updated_sequence",
            params![
                finding.review_finding_id,
                finding.project_id.as_str(),
                finding.task_id.as_str(),
                finding.session_id.as_str(),
                finding.run_id.as_ref().map(RunId::as_str),
                finding.tool_call_id.as_ref().map(ToolCallId::as_str),
                finding.workpad_task_id,
                finding.reviewer,
                finding.finding_kind,
                finding.severity,
                finding.summary,
                finding.status,
                finding.evidence_artifact_id,
                finding.follow_up,
                sequence,
            ],
        )?,
        ProjectionRecord::WorkpadIndexReset(reset) => {
            transaction.execute(
                "DELETE FROM workpad_files WHERE project_id = ?1",
                params![reset.project_id.as_str()],
            )?;
            transaction.execute(
                "DELETE FROM workpad_tasks WHERE project_id = ?1",
                params![reset.project_id.as_str()],
            )?
        }
        ProjectionRecord::WorkpadFile(file) => transaction.execute(
            "INSERT INTO workpad_files(
                path, project_id, content_hash, headings, objective, observed_unix,
                updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(path) DO UPDATE SET
                project_id = excluded.project_id,
                content_hash = excluded.content_hash,
                headings = excluded.headings,
                objective = excluded.objective,
                observed_unix = excluded.observed_unix,
                updated_sequence = excluded.updated_sequence",
            params![
                file.path,
                file.project_id.as_str(),
                file.content_hash,
                file.headings,
                file.objective,
                file.observed_unix,
                sequence,
            ],
        )?,
        ProjectionRecord::WorkpadTask(task) => transaction.execute(
            "INSERT INTO workpad_tasks(
                workpad_task_id, project_id, path, source_anchor, title, observed_status,
                capo_execution_status, observed_unix, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(workpad_task_id) DO UPDATE SET
                project_id = excluded.project_id,
                path = excluded.path,
                source_anchor = excluded.source_anchor,
                title = excluded.title,
                observed_status = excluded.observed_status,
                capo_execution_status = excluded.capo_execution_status,
                observed_unix = excluded.observed_unix,
                updated_sequence = excluded.updated_sequence",
            params![
                task.workpad_task_id,
                task.project_id.as_str(),
                task.path,
                task.source_anchor,
                task.title,
                task.observed_status,
                task.capo_execution_status,
                task.observed_unix,
                sequence,
            ],
        )?,
    };
    Ok(())
}
