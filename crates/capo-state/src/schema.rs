use rusqlite::{Connection, Transaction};

use crate::StateResult;

pub(crate) fn migrate(connection: &mut Connection) -> StateResult<()> {
    connection.execute_batch(
        "
        PRAGMA foreign_keys = ON;
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        INSERT OR IGNORE INTO schema_migrations(version) VALUES (1);
        CREATE TABLE IF NOT EXISTS events (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL UNIQUE,
            kind TEXT NOT NULL,
            actor TEXT NOT NULL,
            project_id TEXT,
            task_id TEXT,
            agent_id TEXT,
            session_id TEXT,
            run_id TEXT,
            turn_id TEXT,
            item_id TEXT,
            payload_json TEXT NOT NULL,
            idempotency_key TEXT,
            redaction_state TEXT NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_events_project_idempotency
        ON events(project_id, idempotency_key)
        WHERE project_id IS NOT NULL AND idempotency_key IS NOT NULL;
        CREATE TABLE IF NOT EXISTS projection_records (
            sequence INTEGER NOT NULL,
            projection_kind TEXT NOT NULL,
            record_id TEXT NOT NULL,
            a TEXT,
            b TEXT,
            c TEXT,
            d TEXT,
            e TEXT,
            f TEXT,
            g TEXT,
            h TEXT,
            payload_json TEXT NOT NULL DEFAULT '{}'
        );
        CREATE TABLE IF NOT EXISTS projection_watermarks (
            name TEXT PRIMARY KEY,
            last_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS artifacts (
            artifact_id TEXT PRIMARY KEY,
            project_id TEXT,
            session_id TEXT,
            run_id TEXT,
            kind TEXT NOT NULL,
            uri TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            size_bytes INTEGER NOT NULL,
            redaction_state TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS projects (
            project_id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            status TEXT NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tasks (
            task_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            title TEXT NOT NULL,
            capo_execution_status TEXT NOT NULL,
            active_session_id TEXT,
            latest_summary TEXT,
            evidence_id TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS agents (
            agent_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            name TEXT NOT NULL,
            status TEXT NOT NULL,
            current_session_id TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sessions (
            session_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            task_id TEXT,
            agent_id TEXT NOT NULL,
            title TEXT NOT NULL,
            status TEXT NOT NULL,
            current_goal TEXT NOT NULL,
            latest_summary TEXT,
            latest_confidence INTEGER,
            latest_blocker TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS runs (
            run_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            status TEXT NOT NULL,
            recovery_of_run_id TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS capability_grants (
            capability_grant_id TEXT PRIMARY KEY,
            capability_profile_id TEXT NOT NULL,
            scope_json TEXT NOT NULL,
            effect TEXT NOT NULL,
            subject_json TEXT NOT NULL,
            decision_source TEXT NOT NULL DEFAULT 'unknown',
            persistence TEXT NOT NULL DEFAULT 'unknown',
            explanation TEXT NOT NULL DEFAULT '',
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS permission_approvals (
            approval_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            session_id TEXT,
            tool_call_id TEXT,
            capability_profile_id TEXT NOT NULL,
            scope_json TEXT NOT NULL,
            subject_json TEXT NOT NULL,
            status TEXT NOT NULL,
            requested_by TEXT NOT NULL,
            reason TEXT NOT NULL,
            decision TEXT,
            capability_grant_id TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS connectivity_exposures (
            exposure_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            connectivity_endpoint_id TEXT NOT NULL,
            owner_kind TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            channel_kind TEXT NOT NULL,
            exposure TEXT NOT NULL,
            permission_scope TEXT NOT NULL,
            status TEXT NOT NULL,
            capability_grant_id TEXT,
            health_status TEXT NOT NULL,
            reachable INTEGER NOT NULL,
            revoked_at TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS runtime_targets (
            runtime_target_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            name TEXT NOT NULL,
            runner_kind TEXT NOT NULL,
            workspace_root TEXT NOT NULL,
            artifact_root TEXT NOT NULL,
            default_cwd TEXT NOT NULL,
            capability_profile_id TEXT NOT NULL,
            connectivity_endpoint_id TEXT,
            status TEXT NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS adapter_readiness (
            adapter_kind TEXT NOT NULL,
            project_id TEXT NOT NULL,
            program TEXT NOT NULL,
            opt_in_env TEXT NOT NULL,
            opted_in INTEGER NOT NULL,
            smoke_status TEXT NOT NULL,
            credential_policy TEXT NOT NULL,
            expected_marker TEXT NOT NULL,
            env_allowlist_count INTEGER NOT NULL,
            redaction_rule_count INTEGER NOT NULL,
            output_limit_bytes INTEGER NOT NULL,
            dogfood_blocker TEXT,
            updated_sequence INTEGER NOT NULL,
            PRIMARY KEY(adapter_kind, project_id)
        );
        CREATE TABLE IF NOT EXISTS adapter_smoke_reports (
            smoke_report_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            adapter_kind TEXT NOT NULL,
            smoke_status TEXT NOT NULL,
            credential_scan_status TEXT NOT NULL,
            marker_found INTEGER NOT NULL,
            artifact_root TEXT,
            reason TEXT NOT NULL,
            dogfood_readiness_effect TEXT NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS adapter_dispatch_plans (
            dispatch_plan_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            adapter_kind TEXT NOT NULL,
            provider_kind TEXT NOT NULL,
            credential_scope TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            agent_name TEXT NOT NULL,
            session_id TEXT NOT NULL,
            run_id TEXT NOT NULL,
            runtime_program TEXT NOT NULL,
            runtime_arg_count INTEGER NOT NULL,
            runtime_prompt_policy TEXT NOT NULL,
            runtime_cwd TEXT NOT NULL,
            artifact_root TEXT NOT NULL,
            request_env_count INTEGER NOT NULL,
            env_allowlist_count INTEGER NOT NULL,
            redaction_rule_count INTEGER NOT NULL,
            stdout_format TEXT NOT NULL,
            stderr_policy TEXT NOT NULL,
            provider_cli_executed INTEGER NOT NULL,
            status TEXT NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS adapter_dispatch_gates (
            dispatch_gate_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            dispatch_plan_id TEXT NOT NULL,
            adapter_kind TEXT NOT NULL,
            provider_cli_execution_allowed INTEGER NOT NULL,
            status TEXT NOT NULL,
            required_dogfood_gate TEXT NOT NULL,
            reason_codes TEXT NOT NULL,
            provider_cli_executed INTEGER NOT NULL,
            runtime_prompt_policy TEXT NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS adapter_dispatch_replays (
            dispatch_replay_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            dispatch_plan_id TEXT NOT NULL,
            dispatch_gate_id TEXT NOT NULL,
            adapter_kind TEXT NOT NULL,
            session_id TEXT NOT NULL,
            run_id TEXT NOT NULL,
            fixture_path TEXT NOT NULL,
            fixture_hash TEXT NOT NULL,
            input_event_count INTEGER NOT NULL,
            appended_event_count INTEGER NOT NULL,
            tool_event_count INTEGER NOT NULL,
            summary_event_count INTEGER NOT NULL,
            completed_turn_count INTEGER NOT NULL,
            provider_cli_executed INTEGER NOT NULL,
            raw_content_policy TEXT NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS adapter_dispatch_execution_requests (
            execution_request_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            dispatch_plan_id TEXT NOT NULL,
            dispatch_gate_id TEXT NOT NULL,
            adapter_kind TEXT NOT NULL,
            provider_cli_execution_allowed INTEGER NOT NULL,
            provider_cli_executed INTEGER NOT NULL,
            status TEXT NOT NULL,
            opt_in_env TEXT NOT NULL,
            runtime_prompt_policy TEXT NOT NULL,
            reason_codes TEXT NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS adapter_dispatch_executions (
            dispatch_execution_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            dispatch_plan_id TEXT NOT NULL,
            execution_request_id TEXT NOT NULL,
            adapter_kind TEXT NOT NULL,
            session_id TEXT NOT NULL,
            run_id TEXT NOT NULL,
            provider_cli_execution_allowed INTEGER NOT NULL,
            provider_cli_executed INTEGER NOT NULL,
            status TEXT NOT NULL,
            exit_code INTEGER,
            runtime_process_ref TEXT,
            stdout_artifact_id TEXT,
            stderr_artifact_id TEXT,
            artifact_root TEXT NOT NULL,
            credential_scan_status TEXT NOT NULL,
            raw_prompt_policy TEXT NOT NULL,
            raw_output_policy TEXT NOT NULL,
            reason_codes TEXT NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS adapter_dispatch_prompt_sources (
            prompt_source_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            dispatch_plan_id TEXT NOT NULL,
            prompt_hash TEXT NOT NULL,
            source_kind TEXT NOT NULL,
            source_ref TEXT,
            source_hash TEXT,
            materialization_status TEXT NOT NULL,
            raw_prompt_policy TEXT NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS adapter_dispatch_prompt_materializations (
            materialization_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            dispatch_plan_id TEXT NOT NULL,
            prompt_source_id TEXT NOT NULL,
            source_kind TEXT NOT NULL,
            source_ref TEXT,
            expected_source_hash TEXT,
            observed_source_hash TEXT,
            expected_prompt_hash TEXT NOT NULL,
            materialized_prompt_hash TEXT,
            status TEXT NOT NULL,
            raw_prompt_policy TEXT NOT NULL,
            reason_codes TEXT NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tool_calls (
            tool_call_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            turn_id TEXT,
            tool_name TEXT NOT NULL,
            tool_origin TEXT NOT NULL,
            status TEXT NOT NULL,
            input_artifact_id TEXT,
            output_artifact_id TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tool_observations (
            tool_observation_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            tool_call_id TEXT,
            source TEXT NOT NULL,
            external_tool_ref TEXT,
            tool_name TEXT NOT NULL,
            observed_status TEXT NOT NULL,
            instrumentation_level TEXT NOT NULL,
            confidence TEXT NOT NULL,
            raw_event_hash TEXT NOT NULL,
            artifact_id TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS memory_packet_refs (
            memory_packet_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            task_id TEXT,
            agent_id TEXT,
            session_id TEXT,
            run_id TEXT,
            turn_id TEXT,
            packet_artifact_id TEXT,
            purpose TEXT NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS memory_records (
            memory_record_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            scope TEXT NOT NULL,
            scope_owner_ref TEXT NOT NULL,
            subject_ref TEXT,
            sensitivity_classification TEXT NOT NULL,
            record_kind TEXT NOT NULL,
            subject TEXT NOT NULL,
            predicate TEXT NOT NULL,
            object TEXT NOT NULL,
            body TEXT NOT NULL,
            confidence TEXT NOT NULL,
            review_state TEXT NOT NULL,
            source_count INTEGER NOT NULL,
            valid_from TEXT,
            valid_until TEXT,
            supersedes_memory_record_id TEXT,
            revoked_by_memory_record_id TEXT,
            redaction_state TEXT NOT NULL,
            invalidated_at TEXT,
            invalidation_reason TEXT,
            packet_item_ref TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS memory_sources (
            memory_source_id TEXT PRIMARY KEY,
            memory_record_id TEXT NOT NULL,
            source_kind TEXT NOT NULL,
            source_event_id TEXT,
            source_artifact_id TEXT,
            source_path TEXT,
            source_anchor TEXT,
            source_content_hash TEXT,
            source_sequence INTEGER,
            quote_artifact_id TEXT,
            observed_at TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS evidence (
            evidence_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            task_id TEXT,
            session_id TEXT,
            run_id TEXT,
            kind TEXT NOT NULL,
            artifact_id TEXT,
            confidence INTEGER NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS task_outcome_reports (
            task_outcome_report_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            task_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            run_id TEXT NOT NULL,
            outcome_status TEXT NOT NULL,
            started_sequence INTEGER NOT NULL,
            completed_sequence INTEGER NOT NULL,
            duration_sequence_span INTEGER NOT NULL,
            action_count INTEGER NOT NULL,
            tool_call_count INTEGER NOT NULL,
            evidence_count INTEGER NOT NULL,
            memory_packet_count INTEGER NOT NULL,
            confidence INTEGER,
            blocker TEXT,
            review_outcome TEXT NOT NULL,
            report_artifact_id TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS review_findings (
            review_finding_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            task_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            run_id TEXT,
            tool_call_id TEXT,
            workpad_task_id TEXT,
            reviewer TEXT NOT NULL,
            finding_kind TEXT NOT NULL,
            severity TEXT NOT NULL,
            summary TEXT NOT NULL,
            status TEXT NOT NULL,
            evidence_artifact_id TEXT,
            follow_up TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS workpad_files (
            path TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            headings TEXT NOT NULL,
            objective TEXT,
            observed_unix INTEGER NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS workpad_tasks (
            workpad_task_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            path TEXT NOT NULL,
            source_anchor TEXT NOT NULL,
            title TEXT NOT NULL,
            observed_status TEXT NOT NULL,
            capo_execution_status TEXT NOT NULL,
            observed_unix INTEGER NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS recovery_attempts (
            recovery_attempt_id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            started_sequence INTEGER NOT NULL,
            completed_sequence INTEGER,
            notes TEXT NOT NULL
        );
        ",
    )?;
    add_missing_column(
        connection,
        "capability_grants",
        "decision_source",
        "TEXT NOT NULL DEFAULT 'unknown'",
    )?;
    add_missing_column(
        connection,
        "capability_grants",
        "persistence",
        "TEXT NOT NULL DEFAULT 'unknown'",
    )?;
    add_missing_column(
        connection,
        "capability_grants",
        "explanation",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    Ok(())
}

fn add_missing_column(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> StateResult<()> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !columns.iter().any(|existing| existing == column) {
        connection.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )?;
    }
    Ok(())
}

pub(crate) fn clear_projection_tables(transaction: &Transaction<'_>) -> StateResult<()> {
    for table in [
        "projects",
        "tasks",
        "agents",
        "sessions",
        "runs",
        "capability_grants",
        "permission_approvals",
        "connectivity_exposures",
        "runtime_targets",
        "adapter_readiness",
        "adapter_smoke_reports",
        "adapter_dispatch_plans",
        "adapter_dispatch_gates",
        "adapter_dispatch_replays",
        "adapter_dispatch_execution_requests",
        "adapter_dispatch_executions",
        "adapter_dispatch_prompt_sources",
        "adapter_dispatch_prompt_materializations",
        "tool_calls",
        "tool_observations",
        "memory_packet_refs",
        "memory_records",
        "memory_sources",
        "evidence",
        "task_outcome_reports",
        "review_findings",
        "workpad_files",
        "workpad_tasks",
        "projection_watermarks",
    ] {
        transaction.execute(&format!("DELETE FROM {table}"), [])?;
    }
    Ok(())
}
