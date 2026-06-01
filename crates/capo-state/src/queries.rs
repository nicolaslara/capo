use capo_core::{
    AgentId, EvidenceId, GoalId, MemoryPacketId, ProjectId, RequirementId, RunId, SessionId,
    TaskId, ToolCallId,
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    AdapterDispatchExecutionProjection, AdapterDispatchExecutionRequestProjection,
    AdapterDispatchGateProjection, AdapterDispatchPlanProjection,
    AdapterDispatchPromptMaterializationProjection, AdapterDispatchPromptSourceProjection,
    AdapterDispatchReplayProjection, AdapterReadinessProjection, AdapterSmokeReportProjection,
    AgentProjection, ArtifactRecord, CapabilityGrantProjection, CheckpointProjection,
    ConnectivityExposureProjection, DelegatedProviderGoalProjection, EventKind, EventRecord,
    EvidenceProjection, GoalContinuationProjection, GoalProjection, GoalReportProjection,
    InFlightRun, MemoryPacketProjection, MemoryRecordProjection, MemorySourceProjection,
    PermissionApprovalProjection, RedactionState, RequirementLedgerProjection,
    ReviewFindingProjection, RunProjection, RunScoreProjection, RuntimeTargetProjection,
    SessionProjection, SourceBindingProjection, SqliteStateStore, StateError, StateResult,
    TaskOutcomeReportProjection, TaskProjection, ToolCallProjection, ToolCallProvenance,
    ToolObservationProjection, WorkpadFileProjection, WorkpadTaskProjection,
    WorkspaceLeaseProjection, optional_id,
};

impl SqliteStateStore {
    pub fn watermark(&self, name: &str) -> StateResult<Option<i64>> {
        let connection = Connection::open(&self.db_path)?;
        let watermark = connection
            .query_row(
                "SELECT last_sequence FROM projection_watermarks WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .optional()?;
        Ok(watermark)
    }

    pub fn session(&self, session_id: &SessionId) -> StateResult<Option<SessionProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let session = connection
            .query_row(
                "SELECT session_id, project_id, task_id, agent_id, title, status, current_goal,
                        latest_summary, latest_confidence, latest_blocker, external_session_ref,
                        updated_sequence
                 FROM sessions
                 WHERE session_id = ?1",
                params![session_id.as_str()],
                |row| {
                    Ok(SessionProjection {
                        session_id: SessionId::new(row.get::<_, String>(0)?),
                        project_id: ProjectId::new(row.get::<_, String>(1)?),
                        task_id: optional_id(row.get::<_, Option<String>>(2)?),
                        agent_id: AgentId::new(row.get::<_, String>(3)?),
                        title: row.get(4)?,
                        status: row.get(5)?,
                        current_goal: row.get(6)?,
                        latest_summary: row.get(7)?,
                        latest_confidence: row.get(8)?,
                        latest_blocker: row.get(9)?,
                        external_session_ref: row.get(10)?,
                        updated_sequence: row.get(11)?,
                    })
                },
            )
            .optional()?;
        Ok(session)
    }

    pub fn task(&self, task_id: &TaskId) -> StateResult<Option<TaskProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let task = connection
            .query_row(
                "SELECT task_id, project_id, title, capo_execution_status, active_session_id,
                        latest_summary, evidence_id, updated_sequence
                 FROM tasks
                 WHERE task_id = ?1",
                params![task_id.as_str()],
                |row| {
                    Ok(TaskProjection {
                        task_id: TaskId::new(row.get::<_, String>(0)?),
                        project_id: ProjectId::new(row.get::<_, String>(1)?),
                        title: row.get(2)?,
                        capo_execution_status: row.get(3)?,
                        active_session_id: optional_id(row.get::<_, Option<String>>(4)?),
                        latest_summary: row.get(5)?,
                        evidence_id: optional_id(row.get::<_, Option<String>>(6)?),
                        updated_sequence: row.get(7)?,
                    })
                },
            )
            .optional()?;
        Ok(task)
    }

    pub fn agent(&self, agent_id: &AgentId) -> StateResult<Option<AgentProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let agent = connection
            .query_row(
                "SELECT agent_id, project_id, name, status, current_session_id, updated_sequence
                 FROM agents
                 WHERE agent_id = ?1",
                params![agent_id.as_str()],
                |row| {
                    Ok(AgentProjection {
                        agent_id: AgentId::new(row.get::<_, String>(0)?),
                        project_id: ProjectId::new(row.get::<_, String>(1)?),
                        name: row.get(2)?,
                        status: row.get(3)?,
                        current_session_id: optional_id(row.get::<_, Option<String>>(4)?),
                        updated_sequence: row.get(5)?,
                    })
                },
            )
            .optional()?;
        Ok(agent)
    }

    pub fn agent_by_name(&self, name: &str) -> StateResult<Option<AgentProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let agent = connection
            .query_row(
                "SELECT agent_id, project_id, name, status, current_session_id, updated_sequence
                 FROM agents
                 WHERE name = ?1
                 ORDER BY updated_sequence DESC
                 LIMIT 1",
                params![name],
                |row| {
                    Ok(AgentProjection {
                        agent_id: AgentId::new(row.get::<_, String>(0)?),
                        project_id: ProjectId::new(row.get::<_, String>(1)?),
                        name: row.get(2)?,
                        status: row.get(3)?,
                        current_session_id: optional_id(row.get::<_, Option<String>>(4)?),
                        updated_sequence: row.get(5)?,
                    })
                },
            )
            .optional()?;
        Ok(agent)
    }

    pub fn agents(&self) -> StateResult<Vec<AgentProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT agent_id, project_id, name, status, current_session_id, updated_sequence
             FROM agents
             ORDER BY name ASC, agent_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(AgentProjection {
                agent_id: AgentId::new(row.get::<_, String>(0)?),
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                name: row.get(2)?,
                status: row.get(3)?,
                current_session_id: optional_id(row.get::<_, Option<String>>(4)?),
                updated_sequence: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn run(&self, run_id: &RunId) -> StateResult<Option<RunProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let run = connection
            .query_row(
                "SELECT run_id, session_id, status, recovery_of_run_id, updated_sequence
                 FROM runs
                 WHERE run_id = ?1",
                params![run_id.as_str()],
                |row| {
                    Ok(RunProjection {
                        run_id: RunId::new(row.get::<_, String>(0)?),
                        session_id: SessionId::new(row.get::<_, String>(1)?),
                        status: row.get(2)?,
                        recovery_of_run_id: optional_id(row.get::<_, Option<String>>(3)?),
                        updated_sequence: row.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(run)
    }

    pub fn run_for_session(&self, session_id: &SessionId) -> StateResult<Option<RunProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let run = connection
            .query_row(
                "SELECT run_id, session_id, status, recovery_of_run_id, updated_sequence
                 FROM runs
                 WHERE session_id = ?1
                 ORDER BY updated_sequence DESC
                 LIMIT 1",
                params![session_id.as_str()],
                |row| {
                    Ok(RunProjection {
                        run_id: RunId::new(row.get::<_, String>(0)?),
                        session_id: SessionId::new(row.get::<_, String>(1)?),
                        status: row.get(2)?,
                        recovery_of_run_id: optional_id(row.get::<_, Option<String>>(3)?),
                        updated_sequence: row.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(run)
    }

    pub fn active_looking_runs(&self) -> StateResult<Vec<RunProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT run_id, session_id, status, recovery_of_run_id, updated_sequence
             FROM runs
             WHERE status IN ('starting', 'running', 'stopping', 'active')
             ORDER BY updated_sequence ASC, run_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(RunProjection {
                run_id: RunId::new(row.get::<_, String>(0)?),
                session_id: SessionId::new(row.get::<_, String>(1)?),
                status: row.get(2)?,
                recovery_of_run_id: optional_id(row.get::<_, Option<String>>(3)?),
                updated_sequence: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn active_looking_runs_for_project(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<RunProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT runs.run_id, runs.session_id, runs.status, runs.recovery_of_run_id,
                    runs.updated_sequence
             FROM runs
             JOIN sessions ON sessions.session_id = runs.session_id
             WHERE sessions.project_id = ?1
                AND runs.status IN ('starting', 'running', 'stopping', 'active')
             ORDER BY runs.updated_sequence ASC, runs.run_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(RunProjection {
                run_id: RunId::new(row.get::<_, String>(0)?),
                session_id: SessionId::new(row.get::<_, String>(1)?),
                status: row.get(2)?,
                recovery_of_run_id: optional_id(row.get::<_, Option<String>>(3)?),
                updated_sequence: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    /// In-flight runs for crash recovery: every active-looking run in the
    /// project, paired with the PID/process-group reference its spawning side
    /// persisted before the spawn returned (RTL10).
    ///
    /// The PID is read from the latest `run.started` event for the run that
    /// carries an `external_pid` in its payload (the in-flight marker the live
    /// path persists before waiting). A run with no such marker (a
    /// deterministic/mock run that never spawned a process) reports
    /// `external_pid: None` and reaps as "no process to reap".
    pub fn inflight_runs_for_project(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<InFlightRun>> {
        let active = self.active_looking_runs_for_project(project_id)?;
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT payload_json
             FROM events
             WHERE run_id = ?1 AND kind = 'run.started'
             ORDER BY sequence DESC",
        )?;
        let mut inflight = Vec::with_capacity(active.len());
        for run in active {
            let mut external_pid = None;
            let mut boot_id = None;
            let mut runtime_process_ref = None;
            let mut rows =
                statement.query_map(params![run.run_id.as_str()], |row| row.get::<_, String>(0))?;
            for payload in rows.by_ref() {
                let payload = payload?;
                if let Some(found) = parse_inflight_marker(&payload) {
                    external_pid = found.external_pid;
                    boot_id = found.boot_id;
                    runtime_process_ref = found.runtime_process_ref;
                    if external_pid.is_some() {
                        break;
                    }
                }
            }
            inflight.push(InFlightRun {
                run_id: run.run_id,
                session_id: run.session_id,
                status: run.status,
                external_pid,
                boot_id,
                runtime_process_ref,
            });
        }
        Ok(inflight)
    }

    pub fn capability_grants(&self) -> StateResult<Vec<CapabilityGrantProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT capability_grant_id, capability_profile_id, scope_json, effect,
                    subject_json, decision_source, persistence, explanation,
                    created_at, expires_at, revoked_at, updated_sequence
             FROM capability_grants
             ORDER BY updated_sequence ASC, capability_grant_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(CapabilityGrantProjection {
                capability_grant_id: row.get(0)?,
                capability_profile_id: row.get(1)?,
                scope_json: row.get(2)?,
                effect: row.get(3)?,
                subject_json: row.get(4)?,
                decision_source: row.get(5)?,
                persistence: row.get(6)?,
                explanation: row.get(7)?,
                created_at: row.get(8)?,
                expires_at: row.get(9)?,
                revoked_at: row.get(10)?,
                updated_sequence: row.get(11)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    /// SG3: read a single durable grant by id, or `None` if absent.
    ///
    /// The grant-revoke flow loads the existing grant so the
    /// `capability.grant_revoked` projection can re-emit the SAME grant body
    /// (scope/effect/subject/...) with only `revoked_at` set, and the decide-step
    /// read-back consults the loaded grant's `revoked_at`/`expires_at`.
    pub fn capability_grant_by_id(
        &self,
        capability_grant_id: &str,
    ) -> StateResult<Option<CapabilityGrantProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT capability_grant_id, capability_profile_id, scope_json, effect,
                    subject_json, decision_source, persistence, explanation,
                    created_at, expires_at, revoked_at, updated_sequence
             FROM capability_grants
             WHERE capability_grant_id = ?1",
        )?;
        let grant = statement
            .query_row(params![capability_grant_id], |row| {
                Ok(CapabilityGrantProjection {
                    capability_grant_id: row.get(0)?,
                    capability_profile_id: row.get(1)?,
                    scope_json: row.get(2)?,
                    effect: row.get(3)?,
                    subject_json: row.get(4)?,
                    decision_source: row.get(5)?,
                    persistence: row.get(6)?,
                    explanation: row.get(7)?,
                    created_at: row.get(8)?,
                    expires_at: row.get(9)?,
                    revoked_at: row.get(10)?,
                    updated_sequence: row.get(11)?,
                })
            })
            .optional()?;
        Ok(grant)
    }

    /// SG5: read a single workspace lease by its key (the canonicalized
    /// workspace root), or `None` if no lease was ever acquired for that key.
    ///
    /// The single-writer lock consults this BEFORE granting a write lease: a row
    /// whose `status` is `held` (and `released_at` is null) means a writer
    /// already owns the workspace, so a second writer is rejected. Acquire and
    /// release both upsert this same row, so it rebuilds from the event log.
    pub fn workspace_lease_by_id(
        &self,
        workspace_lease_id: &str,
    ) -> StateResult<Option<WorkspaceLeaseProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT workspace_lease_id, project_id, holder_session_id, holder_run_id,
                    status, acquired_at, released_at, release_reason, updated_sequence
             FROM workspace_leases
             WHERE workspace_lease_id = ?1",
        )?;
        let lease = statement
            .query_row(params![workspace_lease_id], |row| {
                Ok(WorkspaceLeaseProjection {
                    workspace_lease_id: row.get(0)?,
                    project_id: ProjectId::new(row.get::<_, String>(1)?),
                    holder_session_id: SessionId::new(row.get::<_, String>(2)?),
                    holder_run_id: optional_id(row.get::<_, Option<String>>(3)?),
                    status: row.get(4)?,
                    acquired_at: row.get(5)?,
                    released_at: row.get(6)?,
                    release_reason: row.get(7)?,
                    updated_sequence: row.get(8)?,
                })
            })
            .optional()?;
        Ok(lease)
    }

    /// SG5: every workspace lease row for a project, oldest first.
    pub fn workspace_leases(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<WorkspaceLeaseProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT workspace_lease_id, project_id, holder_session_id, holder_run_id,
                    status, acquired_at, released_at, release_reason, updated_sequence
             FROM workspace_leases
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, workspace_lease_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(WorkspaceLeaseProjection {
                workspace_lease_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                holder_session_id: SessionId::new(row.get::<_, String>(2)?),
                holder_run_id: optional_id(row.get::<_, Option<String>>(3)?),
                status: row.get(4)?,
                acquired_at: row.get(5)?,
                released_at: row.get(6)?,
                release_reason: row.get(7)?,
                updated_sequence: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    /// SG8: read one shadow-git checkpoint back by id (`None` when absent), so the
    /// controller can read back the restorable commit ref before a `Restore`.
    pub fn checkpoint_by_id(
        &self,
        checkpoint_id: &str,
    ) -> StateResult<Option<CheckpointProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT checkpoint_id, project_id, session_id, run_id, turn_id, kind,
                    commit_ref, workspace_root, shadow_git_dir, content_hash,
                    created_at, restored_at, updated_sequence
             FROM checkpoints
             WHERE checkpoint_id = ?1",
        )?;
        let checkpoint = statement
            .query_row(params![checkpoint_id], checkpoint_from_row)
            .optional()?;
        Ok(checkpoint)
    }

    /// SG8: every checkpoint for a run, oldest first -- the per-turn checkpoint
    /// ring the loop takes and the recovery/audit path reads.
    pub fn checkpoints_for_run(&self, run_id: &RunId) -> StateResult<Vec<CheckpointProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT checkpoint_id, project_id, session_id, run_id, turn_id, kind,
                    commit_ref, workspace_root, shadow_git_dir, content_hash,
                    created_at, restored_at, updated_sequence
             FROM checkpoints
             WHERE run_id = ?1
             ORDER BY updated_sequence ASC, checkpoint_id ASC",
        )?;
        let rows = statement.query_map(params![run_id.as_str()], checkpoint_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn permission_approvals(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<PermissionApprovalProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT approval_id, project_id, session_id, tool_call_id, capability_profile_id,
                    scope_json, subject_json, status, requested_by, reason, decision,
                    capability_grant_id, updated_sequence
             FROM permission_approvals
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, approval_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(PermissionApprovalProjection {
                approval_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                session_id: optional_id(row.get::<_, Option<String>>(2)?),
                tool_call_id: optional_id(row.get::<_, Option<String>>(3)?),
                capability_profile_id: row.get(4)?,
                scope_json: row.get(5)?,
                subject_json: row.get(6)?,
                status: row.get(7)?,
                requested_by: row.get(8)?,
                reason: row.get(9)?,
                decision: row.get(10)?,
                capability_grant_id: row.get(11)?,
                updated_sequence: row.get(12)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn permission_approval(
        &self,
        project_id: &ProjectId,
        approval_id: &str,
    ) -> StateResult<Option<PermissionApprovalProjection>> {
        Ok(self
            .permission_approvals(project_id)?
            .into_iter()
            .find(|approval| approval.approval_id == approval_id))
    }

    pub fn connectivity_exposures(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<ConnectivityExposureProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT exposure_id, project_id, connectivity_endpoint_id, owner_kind, owner_id,
                    channel_kind, exposure, permission_scope, status, capability_grant_id,
                    health_status, reachable, revoked_at, updated_sequence
             FROM connectivity_exposures
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, exposure_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(ConnectivityExposureProjection {
                exposure_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                connectivity_endpoint_id: row.get(2)?,
                owner_kind: row.get(3)?,
                owner_id: row.get(4)?,
                channel_kind: row.get(5)?,
                exposure: row.get(6)?,
                permission_scope: row.get(7)?,
                status: row.get(8)?,
                capability_grant_id: row.get(9)?,
                health_status: row.get(10)?,
                reachable: row.get::<_, i64>(11)? != 0,
                revoked_at: row.get(12)?,
                updated_sequence: row.get(13)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn runtime_targets(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<RuntimeTargetProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT runtime_target_id, project_id, name, runner_kind, workspace_root,
                    artifact_root, default_cwd, capability_profile_id, connectivity_endpoint_id,
                    status, updated_sequence
             FROM runtime_targets
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, runtime_target_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(RuntimeTargetProjection {
                runtime_target_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                name: row.get(2)?,
                runner_kind: row.get(3)?,
                workspace_root: row.get(4)?,
                artifact_root: row.get(5)?,
                default_cwd: row.get(6)?,
                capability_profile_id: row.get(7)?,
                connectivity_endpoint_id: row.get(8)?,
                status: row.get(9)?,
                updated_sequence: row.get(10)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn adapter_readiness(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<AdapterReadinessProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT adapter_kind, project_id, program, opt_in_env, opted_in, smoke_status,
                    credential_policy, expected_marker, env_allowlist_count,
                    redaction_rule_count, output_limit_bytes, dogfood_blocker, updated_sequence
             FROM adapter_readiness
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, adapter_kind ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(AdapterReadinessProjection {
                adapter_kind: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                program: row.get(2)?,
                opt_in_env: row.get(3)?,
                opted_in: row.get::<_, i64>(4)? != 0,
                smoke_status: row.get(5)?,
                credential_policy: row.get(6)?,
                expected_marker: row.get(7)?,
                env_allowlist_count: row.get(8)?,
                redaction_rule_count: row.get(9)?,
                output_limit_bytes: row.get(10)?,
                dogfood_blocker: row.get(11)?,
                updated_sequence: row.get(12)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn adapter_smoke_reports(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<AdapterSmokeReportProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT smoke_report_id, project_id, adapter_kind, smoke_status,
                    credential_scan_status, marker_found, artifact_root, reason,
                    dogfood_readiness_effect, updated_sequence
             FROM adapter_smoke_reports
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, smoke_report_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(AdapterSmokeReportProjection {
                smoke_report_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                adapter_kind: row.get(2)?,
                smoke_status: row.get(3)?,
                credential_scan_status: row.get(4)?,
                marker_found: row.get::<_, i64>(5)? != 0,
                artifact_root: row.get(6)?,
                reason: row.get(7)?,
                dogfood_readiness_effect: row.get(8)?,
                updated_sequence: row.get(9)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn adapter_dispatch_plans(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<AdapterDispatchPlanProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT dispatch_plan_id, project_id, adapter_kind, provider_kind,
                    credential_scope, agent_id, agent_name, session_id, run_id,
                    runtime_program, runtime_arg_count, runtime_prompt_policy,
                    runtime_cwd, artifact_root, request_env_count, env_allowlist_count,
                    redaction_rule_count, stdout_format, stderr_policy,
                    provider_cli_executed, status, updated_sequence
             FROM adapter_dispatch_plans
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, dispatch_plan_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(AdapterDispatchPlanProjection {
                dispatch_plan_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                adapter_kind: row.get(2)?,
                provider_kind: row.get(3)?,
                credential_scope: row.get(4)?,
                agent_id: AgentId::new(row.get::<_, String>(5)?),
                agent_name: row.get(6)?,
                session_id: SessionId::new(row.get::<_, String>(7)?),
                run_id: RunId::new(row.get::<_, String>(8)?),
                runtime_program: row.get(9)?,
                runtime_arg_count: row.get(10)?,
                runtime_prompt_policy: row.get(11)?,
                runtime_cwd: row.get(12)?,
                artifact_root: row.get(13)?,
                request_env_count: row.get(14)?,
                env_allowlist_count: row.get(15)?,
                redaction_rule_count: row.get(16)?,
                stdout_format: row.get(17)?,
                stderr_policy: row.get(18)?,
                provider_cli_executed: row.get::<_, i64>(19)? != 0,
                status: row.get(20)?,
                updated_sequence: row.get(21)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn adapter_dispatch_gates(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<AdapterDispatchGateProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT dispatch_gate_id, project_id, dispatch_plan_id, adapter_kind,
                    provider_cli_execution_allowed, status, required_dogfood_gate,
                    reason_codes, provider_cli_executed, runtime_prompt_policy,
                    updated_sequence
             FROM adapter_dispatch_gates
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, dispatch_gate_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(AdapterDispatchGateProjection {
                dispatch_gate_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                dispatch_plan_id: row.get(2)?,
                adapter_kind: row.get(3)?,
                provider_cli_execution_allowed: row.get::<_, i64>(4)? != 0,
                status: row.get(5)?,
                required_dogfood_gate: row.get(6)?,
                reason_codes: row.get(7)?,
                provider_cli_executed: row.get::<_, i64>(8)? != 0,
                runtime_prompt_policy: row.get(9)?,
                updated_sequence: row.get(10)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn adapter_dispatch_replays(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<AdapterDispatchReplayProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT dispatch_replay_id, project_id, dispatch_plan_id, dispatch_gate_id,
                    adapter_kind, session_id, run_id, fixture_path, fixture_hash,
                    input_event_count, appended_event_count, tool_event_count,
                    summary_event_count, completed_turn_count, provider_cli_executed,
                    raw_content_policy, updated_sequence
             FROM adapter_dispatch_replays
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, dispatch_replay_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(AdapterDispatchReplayProjection {
                dispatch_replay_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                dispatch_plan_id: row.get(2)?,
                dispatch_gate_id: row.get(3)?,
                adapter_kind: row.get(4)?,
                session_id: SessionId::new(row.get::<_, String>(5)?),
                run_id: RunId::new(row.get::<_, String>(6)?),
                fixture_path: row.get(7)?,
                fixture_hash: row.get(8)?,
                input_event_count: row.get(9)?,
                appended_event_count: row.get(10)?,
                tool_event_count: row.get(11)?,
                summary_event_count: row.get(12)?,
                completed_turn_count: row.get(13)?,
                provider_cli_executed: row.get::<_, i64>(14)? != 0,
                raw_content_policy: row.get(15)?,
                updated_sequence: row.get(16)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn adapter_dispatch_execution_requests(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<AdapterDispatchExecutionRequestProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT execution_request_id, project_id, dispatch_plan_id, dispatch_gate_id,
                    adapter_kind, provider_cli_execution_allowed, provider_cli_executed,
                    status, opt_in_env, runtime_prompt_policy, reason_codes, updated_sequence
             FROM adapter_dispatch_execution_requests
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, execution_request_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(AdapterDispatchExecutionRequestProjection {
                execution_request_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                dispatch_plan_id: row.get(2)?,
                dispatch_gate_id: row.get(3)?,
                adapter_kind: row.get(4)?,
                provider_cli_execution_allowed: row.get::<_, i64>(5)? != 0,
                provider_cli_executed: row.get::<_, i64>(6)? != 0,
                status: row.get(7)?,
                opt_in_env: row.get(8)?,
                runtime_prompt_policy: row.get(9)?,
                reason_codes: row.get(10)?,
                updated_sequence: row.get(11)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn adapter_dispatch_executions(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<AdapterDispatchExecutionProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT dispatch_execution_id, project_id, dispatch_plan_id,
                    execution_request_id, adapter_kind, session_id, run_id,
                    provider_cli_execution_allowed, provider_cli_executed, status,
                    exit_code, runtime_process_ref, stdout_artifact_id,
                    stderr_artifact_id, artifact_root, credential_scan_status,
                    raw_prompt_policy, raw_output_policy, reason_codes, updated_sequence
             FROM adapter_dispatch_executions
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, dispatch_execution_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(AdapterDispatchExecutionProjection {
                dispatch_execution_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                dispatch_plan_id: row.get(2)?,
                execution_request_id: row.get(3)?,
                adapter_kind: row.get(4)?,
                session_id: SessionId::new(row.get::<_, String>(5)?),
                run_id: RunId::new(row.get::<_, String>(6)?),
                provider_cli_execution_allowed: row.get::<_, i64>(7)? != 0,
                provider_cli_executed: row.get::<_, i64>(8)? != 0,
                status: row.get(9)?,
                exit_code: row.get(10)?,
                runtime_process_ref: row.get(11)?,
                stdout_artifact_id: row.get(12)?,
                stderr_artifact_id: row.get(13)?,
                artifact_root: row.get(14)?,
                credential_scan_status: row.get(15)?,
                raw_prompt_policy: row.get(16)?,
                raw_output_policy: row.get(17)?,
                reason_codes: row.get(18)?,
                updated_sequence: row.get(19)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn adapter_dispatch_prompt_sources(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<AdapterDispatchPromptSourceProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT prompt_source_id, project_id, dispatch_plan_id, prompt_hash,
                    source_kind, source_ref, source_hash, materialization_status,
                    raw_prompt_policy, updated_sequence
             FROM adapter_dispatch_prompt_sources
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, prompt_source_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(AdapterDispatchPromptSourceProjection {
                prompt_source_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                dispatch_plan_id: row.get(2)?,
                prompt_hash: row.get(3)?,
                source_kind: row.get(4)?,
                source_ref: row.get(5)?,
                source_hash: row.get(6)?,
                materialization_status: row.get(7)?,
                raw_prompt_policy: row.get(8)?,
                updated_sequence: row.get(9)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn adapter_dispatch_prompt_materializations(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<AdapterDispatchPromptMaterializationProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT materialization_id, project_id, dispatch_plan_id, prompt_source_id,
                    source_kind, source_ref, expected_source_hash, observed_source_hash,
                    expected_prompt_hash, materialized_prompt_hash, status,
                    raw_prompt_policy, reason_codes, updated_sequence
             FROM adapter_dispatch_prompt_materializations
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, materialization_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(AdapterDispatchPromptMaterializationProjection {
                materialization_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                dispatch_plan_id: row.get(2)?,
                prompt_source_id: row.get(3)?,
                source_kind: row.get(4)?,
                source_ref: row.get(5)?,
                expected_source_hash: row.get(6)?,
                observed_source_hash: row.get(7)?,
                expected_prompt_hash: row.get(8)?,
                materialized_prompt_hash: row.get(9)?,
                status: row.get(10)?,
                raw_prompt_policy: row.get(11)?,
                reason_codes: row.get(12)?,
                updated_sequence: row.get(13)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn evidence_for_session(
        &self,
        session_id: &SessionId,
    ) -> StateResult<Vec<EvidenceProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT evidence_id, project_id, task_id, session_id, run_id, kind, artifact_id,
                    confidence, updated_sequence
             FROM evidence
             WHERE session_id = ?1
             ORDER BY updated_sequence ASC, evidence_id ASC",
        )?;
        let rows = statement.query_map(params![session_id.as_str()], |row| {
            Ok(EvidenceProjection {
                evidence_id: EvidenceId::new(row.get::<_, String>(0)?),
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                task_id: optional_id(row.get::<_, Option<String>>(2)?),
                session_id: optional_id(row.get::<_, Option<String>>(3)?),
                run_id: optional_id(row.get::<_, Option<String>>(4)?),
                kind: row.get(5)?,
                artifact_id: row.get(6)?,
                confidence: row.get(7)?,
                updated_sequence: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn project_evidence(&self, project_id: &ProjectId) -> StateResult<Vec<EvidenceProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT evidence_id, project_id, task_id, session_id, run_id, kind, artifact_id,
                    confidence, updated_sequence
             FROM evidence
             WHERE project_id = ?1 AND session_id IS NULL
             ORDER BY updated_sequence ASC, evidence_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(EvidenceProjection {
                evidence_id: EvidenceId::new(row.get::<_, String>(0)?),
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                task_id: optional_id(row.get::<_, Option<String>>(2)?),
                session_id: optional_id(row.get::<_, Option<String>>(3)?),
                run_id: optional_id(row.get::<_, Option<String>>(4)?),
                kind: row.get(5)?,
                artifact_id: row.get(6)?,
                confidence: row.get(7)?,
                updated_sequence: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    /// SG7: a single run-score row by id, or `None` if absent.
    ///
    /// `score_run` reads this back to dedupe an identical re-score and to expose
    /// the durable, queryable outcome (which survives restart because it rebuilds
    /// from the `run.scored` event).
    pub fn run_score_by_id(&self, run_score_id: &str) -> StateResult<Option<RunScoreProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT run_score_id, project_id, task_id, session_id, run_id, outcome, passed,
                    criteria_total, criteria_met, observed_evidence_count, started_at,
                    completed_at, duration_millis, score_inputs_json, updated_sequence
             FROM run_scores
             WHERE run_score_id = ?1",
        )?;
        let score = statement
            .query_row(params![run_score_id], run_score_from_row)
            .optional()?;
        Ok(score)
    }

    /// SG7: every run-score row for a session, oldest first.
    pub fn run_scores_for_session(
        &self,
        session_id: &SessionId,
    ) -> StateResult<Vec<RunScoreProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT run_score_id, project_id, task_id, session_id, run_id, outcome, passed,
                    criteria_total, criteria_met, observed_evidence_count, started_at,
                    completed_at, duration_millis, score_inputs_json, updated_sequence
             FROM run_scores
             WHERE session_id = ?1
             ORDER BY updated_sequence ASC, run_score_id ASC",
        )?;
        let rows = statement.query_map(params![session_id.as_str()], run_score_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn memory_packets_for_session(
        &self,
        session_id: &SessionId,
    ) -> StateResult<Vec<MemoryPacketProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT memory_packet_id, project_id, task_id, agent_id, session_id, run_id,
                    turn_id, packet_artifact_id, purpose, updated_sequence
             FROM memory_packet_refs
             WHERE session_id = ?1
             ORDER BY updated_sequence ASC, memory_packet_id ASC",
        )?;
        let rows = statement.query_map(params![session_id.as_str()], |row| {
            Ok(MemoryPacketProjection {
                memory_packet_id: MemoryPacketId::new(row.get::<_, String>(0)?),
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                task_id: optional_id(row.get::<_, Option<String>>(2)?),
                agent_id: optional_id(row.get::<_, Option<String>>(3)?),
                session_id: optional_id(row.get::<_, Option<String>>(4)?),
                run_id: optional_id(row.get::<_, Option<String>>(5)?),
                turn_id: row.get(6)?,
                packet_artifact_id: row.get(7)?,
                purpose: row.get(8)?,
                updated_sequence: row.get(9)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn memory_records_for_project(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<MemoryRecordProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT memory_record_id, project_id, scope, scope_owner_ref, subject_ref,
                    sensitivity_classification, record_kind, subject, predicate, object,
                    body, confidence, review_state, source_count, valid_from, valid_until,
                    supersedes_memory_record_id, revoked_by_memory_record_id, redaction_state,
                    invalidated_at, invalidation_reason, packet_item_ref, updated_sequence
             FROM memory_records
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, memory_record_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(MemoryRecordProjection {
                memory_record_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                scope: row.get(2)?,
                scope_owner_ref: row.get(3)?,
                subject_ref: row.get(4)?,
                sensitivity_classification: row.get(5)?,
                record_kind: row.get(6)?,
                subject: row.get(7)?,
                predicate: row.get(8)?,
                object: row.get(9)?,
                body: row.get(10)?,
                confidence: row.get(11)?,
                review_state: row.get(12)?,
                source_count: row.get(13)?,
                valid_from: row.get(14)?,
                valid_until: row.get(15)?,
                supersedes_memory_record_id: row.get(16)?,
                revoked_by_memory_record_id: row.get(17)?,
                redaction_state: row.get(18)?,
                invalidated_at: row.get(19)?,
                invalidation_reason: row.get(20)?,
                packet_item_ref: row.get(21)?,
                updated_sequence: row.get(22)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn packet_eligible_memory_records(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<MemoryRecordProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT memory_record_id, project_id, scope, scope_owner_ref, subject_ref,
                    sensitivity_classification, record_kind, subject, predicate, object,
                    body, confidence, review_state, source_count, valid_from, valid_until,
                    supersedes_memory_record_id, revoked_by_memory_record_id, redaction_state,
                    invalidated_at, invalidation_reason, packet_item_ref, updated_sequence
             FROM memory_records
             WHERE project_id = ?1
                AND review_state = 'reviewed'
                AND source_count > 0
                AND valid_until IS NULL
                AND revoked_by_memory_record_id IS NULL
                AND invalidated_at IS NULL
                AND packet_item_ref IS NOT NULL
                AND sensitivity_classification != 'secret_derived'
                AND redaction_state NOT IN ('unknown', 'contains_sensitive')
                AND EXISTS (
                    SELECT 1
                    FROM memory_sources
                    WHERE memory_sources.memory_record_id = memory_records.memory_record_id
                      AND memory_sources.source_content_hash IS NOT NULL
                      AND (
                        memory_sources.source_anchor IS NOT NULL
                        OR memory_sources.source_event_id IS NOT NULL
                        OR memory_sources.source_artifact_id IS NOT NULL
                      )
                )
             ORDER BY updated_sequence ASC, memory_record_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(MemoryRecordProjection {
                memory_record_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                scope: row.get(2)?,
                scope_owner_ref: row.get(3)?,
                subject_ref: row.get(4)?,
                sensitivity_classification: row.get(5)?,
                record_kind: row.get(6)?,
                subject: row.get(7)?,
                predicate: row.get(8)?,
                object: row.get(9)?,
                body: row.get(10)?,
                confidence: row.get(11)?,
                review_state: row.get(12)?,
                source_count: row.get(13)?,
                valid_from: row.get(14)?,
                valid_until: row.get(15)?,
                supersedes_memory_record_id: row.get(16)?,
                revoked_by_memory_record_id: row.get(17)?,
                redaction_state: row.get(18)?,
                invalidated_at: row.get(19)?,
                invalidation_reason: row.get(20)?,
                packet_item_ref: row.get(21)?,
                updated_sequence: row.get(22)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn memory_sources_for_record(
        &self,
        memory_record_id: &str,
    ) -> StateResult<Vec<MemorySourceProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT memory_source_id, memory_record_id, source_kind, source_event_id,
                    source_artifact_id, source_path, source_anchor, source_content_hash,
                    source_sequence, quote_artifact_id, observed_at, updated_sequence
             FROM memory_sources
             WHERE memory_record_id = ?1
             ORDER BY source_sequence ASC, memory_source_id ASC",
        )?;
        let rows = statement.query_map(params![memory_record_id], |row| {
            Ok(MemorySourceProjection {
                memory_source_id: row.get(0)?,
                memory_record_id: row.get(1)?,
                source_kind: row.get(2)?,
                source_event_id: row.get(3)?,
                source_artifact_id: row.get(4)?,
                source_path: row.get(5)?,
                source_anchor: row.get(6)?,
                source_content_hash: row.get(7)?,
                source_sequence: row.get(8)?,
                quote_artifact_id: row.get(9)?,
                observed_at: row.get(10)?,
                updated_sequence: row.get(11)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn task_outcome_reports_for_task(
        &self,
        task_id: &TaskId,
    ) -> StateResult<Vec<TaskOutcomeReportProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT task_outcome_report_id, project_id, task_id, session_id, run_id,
                    outcome_status, started_sequence, completed_sequence,
                    duration_sequence_span, action_count, tool_call_count, evidence_count,
                    memory_packet_count, confidence, blocker, review_outcome, report_artifact_id,
                    updated_sequence
             FROM task_outcome_reports
             WHERE task_id = ?1
             ORDER BY updated_sequence ASC, task_outcome_report_id ASC",
        )?;
        let rows = statement.query_map(params![task_id.as_str()], |row| {
            Ok(TaskOutcomeReportProjection {
                task_outcome_report_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                task_id: TaskId::new(row.get::<_, String>(2)?),
                session_id: SessionId::new(row.get::<_, String>(3)?),
                run_id: RunId::new(row.get::<_, String>(4)?),
                outcome_status: row.get(5)?,
                started_sequence: row.get(6)?,
                completed_sequence: row.get(7)?,
                duration_sequence_span: row.get(8)?,
                action_count: row.get(9)?,
                tool_call_count: row.get(10)?,
                evidence_count: row.get(11)?,
                memory_packet_count: row.get(12)?,
                confidence: row.get(13)?,
                blocker: row.get(14)?,
                review_outcome: row.get(15)?,
                report_artifact_id: row.get(16)?,
                updated_sequence: row.get(17)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn task_outcome_reports(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<TaskOutcomeReportProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT task_outcome_report_id, project_id, task_id, session_id, run_id,
                    outcome_status, started_sequence, completed_sequence,
                    duration_sequence_span, action_count, tool_call_count, evidence_count,
                    memory_packet_count, confidence, blocker, review_outcome, report_artifact_id,
                    updated_sequence
             FROM task_outcome_reports
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, task_outcome_report_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(TaskOutcomeReportProjection {
                task_outcome_report_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                task_id: TaskId::new(row.get::<_, String>(2)?),
                session_id: SessionId::new(row.get::<_, String>(3)?),
                run_id: RunId::new(row.get::<_, String>(4)?),
                outcome_status: row.get(5)?,
                started_sequence: row.get(6)?,
                completed_sequence: row.get(7)?,
                duration_sequence_span: row.get(8)?,
                action_count: row.get(9)?,
                tool_call_count: row.get(10)?,
                evidence_count: row.get(11)?,
                memory_packet_count: row.get(12)?,
                confidence: row.get(13)?,
                blocker: row.get(14)?,
                review_outcome: row.get(15)?,
                report_artifact_id: row.get(16)?,
                updated_sequence: row.get(17)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn task_outcome_reports_for_session(
        &self,
        session_id: &SessionId,
    ) -> StateResult<Vec<TaskOutcomeReportProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT task_outcome_report_id, project_id, task_id, session_id, run_id,
                    outcome_status, started_sequence, completed_sequence,
                    duration_sequence_span, action_count, tool_call_count, evidence_count,
                    memory_packet_count, confidence, blocker, review_outcome, report_artifact_id,
                    updated_sequence
             FROM task_outcome_reports
             WHERE session_id = ?1
             ORDER BY updated_sequence ASC, task_outcome_report_id ASC",
        )?;
        let rows = statement.query_map(params![session_id.as_str()], |row| {
            Ok(TaskOutcomeReportProjection {
                task_outcome_report_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                task_id: TaskId::new(row.get::<_, String>(2)?),
                session_id: SessionId::new(row.get::<_, String>(3)?),
                run_id: RunId::new(row.get::<_, String>(4)?),
                outcome_status: row.get(5)?,
                started_sequence: row.get(6)?,
                completed_sequence: row.get(7)?,
                duration_sequence_span: row.get(8)?,
                action_count: row.get(9)?,
                tool_call_count: row.get(10)?,
                evidence_count: row.get(11)?,
                memory_packet_count: row.get(12)?,
                confidence: row.get(13)?,
                blocker: row.get(14)?,
                review_outcome: row.get(15)?,
                report_artifact_id: row.get(16)?,
                updated_sequence: row.get(17)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn review_findings_for_session(
        &self,
        session_id: &SessionId,
    ) -> StateResult<Vec<ReviewFindingProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT review_finding_id, project_id, task_id, session_id, run_id, tool_call_id,
                    workpad_task_id, reviewer, finding_kind, severity, summary, status,
                    evidence_artifact_id, follow_up, updated_sequence
             FROM review_findings
             WHERE session_id = ?1
             ORDER BY updated_sequence ASC, review_finding_id ASC",
        )?;
        let rows = statement.query_map(params![session_id.as_str()], |row| {
            Ok(ReviewFindingProjection {
                review_finding_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                task_id: TaskId::new(row.get::<_, String>(2)?),
                session_id: SessionId::new(row.get::<_, String>(3)?),
                run_id: optional_id(row.get::<_, Option<String>>(4)?),
                tool_call_id: optional_id(row.get::<_, Option<String>>(5)?),
                workpad_task_id: row.get(6)?,
                reviewer: row.get(7)?,
                finding_kind: row.get(8)?,
                severity: row.get(9)?,
                summary: row.get(10)?,
                status: row.get(11)?,
                evidence_artifact_id: row.get(12)?,
                follow_up: row.get(13)?,
                updated_sequence: row.get(14)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn review_findings(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<ReviewFindingProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT review_finding_id, project_id, task_id, session_id, run_id, tool_call_id,
                    workpad_task_id, reviewer, finding_kind, severity, summary, status,
                    evidence_artifact_id, follow_up, updated_sequence
             FROM review_findings
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, review_finding_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(ReviewFindingProjection {
                review_finding_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                task_id: TaskId::new(row.get::<_, String>(2)?),
                session_id: SessionId::new(row.get::<_, String>(3)?),
                run_id: optional_id(row.get::<_, Option<String>>(4)?),
                tool_call_id: optional_id(row.get::<_, Option<String>>(5)?),
                workpad_task_id: row.get(6)?,
                reviewer: row.get(7)?,
                finding_kind: row.get(8)?,
                severity: row.get(9)?,
                summary: row.get(10)?,
                status: row.get(11)?,
                evidence_artifact_id: row.get(12)?,
                follow_up: row.get(13)?,
                updated_sequence: row.get(14)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn tool_calls_for_session(
        &self,
        session_id: &SessionId,
    ) -> StateResult<Vec<ToolCallProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT tool_call_id, session_id, turn_id, tool_name, tool_origin, status,
                    input_artifact_id, output_artifact_id, correlation_id,
                    permission_decision_id, capability_grant_use_id, started_at,
                    completed_at, updated_sequence
             FROM tool_calls
             WHERE session_id = ?1
             ORDER BY updated_sequence ASC, tool_call_id ASC",
        )?;
        let rows = statement.query_map(params![session_id.as_str()], |row| {
            Ok(ToolCallProjection {
                tool_call_id: ToolCallId::new(row.get::<_, String>(0)?),
                session_id: SessionId::new(row.get::<_, String>(1)?),
                turn_id: row.get(2)?,
                tool_name: row.get(3)?,
                tool_origin: row.get(4)?,
                status: row.get(5)?,
                input_artifact_id: row.get(6)?,
                output_artifact_id: row.get(7)?,
                provenance: ToolCallProvenance {
                    correlation_id: row.get(8)?,
                    permission_decision_id: row.get(9)?,
                    capability_grant_use_id: row.get(10)?,
                    started_at: row.get(11)?,
                    completed_at: row.get(12)?,
                },
                updated_sequence: row.get(13)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn tool_observations_for_session(
        &self,
        session_id: &SessionId,
    ) -> StateResult<Vec<ToolObservationProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT tool_observation_id, session_id, tool_call_id, source, external_tool_ref,
                    tool_name, observed_status, instrumentation_level, confidence,
                    raw_event_hash, artifact_id, updated_sequence
             FROM tool_observations
             WHERE session_id = ?1
             ORDER BY updated_sequence ASC, tool_observation_id ASC",
        )?;
        let rows = statement.query_map(params![session_id.as_str()], |row| {
            Ok(ToolObservationProjection {
                tool_observation_id: row.get(0)?,
                session_id: SessionId::new(row.get::<_, String>(1)?),
                tool_call_id: optional_id(row.get::<_, Option<String>>(2)?),
                source: row.get(3)?,
                external_tool_ref: row.get(4)?,
                tool_name: row.get(5)?,
                observed_status: row.get(6)?,
                instrumentation_level: row.get(7)?,
                confidence: row.get(8)?,
                raw_event_hash: row.get(9)?,
                artifact_id: row.get(10)?,
                updated_sequence: row.get(11)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn workpad_files(&self, project_id: &ProjectId) -> StateResult<Vec<WorkpadFileProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT path, project_id, content_hash, headings, objective, observed_unix, updated_sequence
             FROM workpad_files
             WHERE project_id = ?1
             ORDER BY path ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(WorkpadFileProjection {
                path: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                content_hash: row.get(2)?,
                headings: row.get(3)?,
                objective: row.get(4)?,
                observed_unix: row.get(5)?,
                updated_sequence: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn workpad_file(
        &self,
        project_id: &ProjectId,
        path: &str,
    ) -> StateResult<Option<WorkpadFileProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let file = connection
            .query_row(
                "SELECT path, project_id, content_hash, headings, objective, observed_unix, updated_sequence
                 FROM workpad_files
                 WHERE project_id = ?1 AND path = ?2",
                params![project_id.as_str(), path],
                |row| {
                    Ok(WorkpadFileProjection {
                        path: row.get(0)?,
                        project_id: ProjectId::new(row.get::<_, String>(1)?),
                        content_hash: row.get(2)?,
                        headings: row.get(3)?,
                        objective: row.get(4)?,
                        observed_unix: row.get(5)?,
                        updated_sequence: row.get(6)?,
                    })
                },
            )
            .optional()?;
        Ok(file)
    }

    pub fn workpad_tasks(&self, project_id: &ProjectId) -> StateResult<Vec<WorkpadTaskProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT workpad_task_id, project_id, path, source_anchor, title, observed_status,
                    capo_execution_status, observed_unix, updated_sequence
             FROM workpad_tasks
             WHERE project_id = ?1
             ORDER BY path ASC, source_anchor ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(WorkpadTaskProjection {
                workpad_task_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                path: row.get(2)?,
                source_anchor: row.get(3)?,
                title: row.get(4)?,
                observed_status: row.get(5)?,
                capo_execution_status: row.get(6)?,
                observed_unix: row.get(7)?,
                updated_sequence: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn workpad_task(
        &self,
        project_id: &ProjectId,
        workpad_task_id: &str,
    ) -> StateResult<Option<WorkpadTaskProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let task = connection
            .query_row(
                "SELECT workpad_task_id, project_id, path, source_anchor, title, observed_status,
                        capo_execution_status, observed_unix, updated_sequence
                 FROM workpad_tasks
                 WHERE project_id = ?1 AND workpad_task_id = ?2",
                params![project_id.as_str(), workpad_task_id],
                |row| {
                    Ok(WorkpadTaskProjection {
                        workpad_task_id: row.get(0)?,
                        project_id: ProjectId::new(row.get::<_, String>(1)?),
                        path: row.get(2)?,
                        source_anchor: row.get(3)?,
                        title: row.get(4)?,
                        observed_status: row.get(5)?,
                        capo_execution_status: row.get(6)?,
                        observed_unix: row.get(7)?,
                        updated_sequence: row.get(8)?,
                    })
                },
            )
            .optional()?;
        Ok(task)
    }

    pub fn source_bindings(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<SourceBindingProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT source_binding_id, project_id, task_id, source_kind, source_task_id,
                    source_path, source_anchor, source_hash, binding_status, updated_sequence
             FROM source_bindings
             WHERE project_id = ?1
             ORDER BY task_id ASC, source_path ASC, source_anchor ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], source_binding_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn source_binding_for_task(
        &self,
        task_id: &TaskId,
    ) -> StateResult<Option<SourceBindingProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let binding = connection
            .query_row(
                "SELECT source_binding_id, project_id, task_id, source_kind, source_task_id,
                        source_path, source_anchor, source_hash, binding_status, updated_sequence
                 FROM source_bindings
                 WHERE task_id = ?1
                 ORDER BY updated_sequence DESC
                 LIMIT 1",
                params![task_id.as_str()],
                source_binding_from_row,
            )
            .optional()?;
        Ok(binding)
    }

    pub fn recent_events_for_session(
        &self,
        session_id: &SessionId,
        limit: usize,
    ) -> StateResult<Vec<EventRecord>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT sequence, event_id, kind, actor, project_id, task_id, agent_id, session_id,
                    run_id, turn_id, item_id, payload_json, idempotency_key, redaction_state
             FROM events
             WHERE session_id = ?1
             ORDER BY sequence DESC
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![session_id.as_str(), limit as i64], |row| {
            Ok(EventRecord {
                sequence: row.get(0)?,
                event_id: row.get(1)?,
                kind: row.get(2)?,
                actor: row.get(3)?,
                project_id: optional_id(row.get::<_, Option<String>>(4)?),
                task_id: optional_id(row.get::<_, Option<String>>(5)?),
                agent_id: optional_id(row.get::<_, Option<String>>(6)?),
                session_id: optional_id(row.get::<_, Option<String>>(7)?),
                run_id: optional_id(row.get::<_, Option<String>>(8)?),
                turn_id: row.get(9)?,
                item_id: row.get(10)?,
                payload_json: row.get(11)?,
                idempotency_key: row.get(12)?,
                redaction_state: row.get(13)?,
            })
        })?;
        let mut events = rows.collect::<Result<Vec<_>, _>>()?;
        events.reverse();
        Ok(events)
    }

    /// Events strictly after a caller-supplied sequence watermark, in ascending
    /// sequence order (ST4).
    ///
    /// This is the forward read of the append-only log the streaming event tail
    /// is built on: a subscriber supplies the highest sequence it has already
    /// seen (`from_sequence`) and gets exactly the events committed since, in
    /// order. The store already exposes a monotonic `sequence`
    /// (`last_insert_rowid()` in `append_event`) and a `default` projection
    /// watermark, so this only surfaces a forward read of them; it never returns
    /// the watermark event itself (`sequence > since_sequence`, strictly), so
    /// pairing it with a broadcast subscription that resumes from the same
    /// watermark yields no gap and no duplicate at the seam.
    ///
    /// `limit` bounds the catch-up batch so a subscriber reconnecting against a
    /// very long log reads a bounded page rather than the whole history in one
    /// query; callers page by advancing `from_sequence` to the last returned
    /// sequence.
    pub fn events_after(&self, since_sequence: i64, limit: usize) -> StateResult<Vec<EventRecord>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT sequence, event_id, kind, actor, project_id, task_id, agent_id, session_id,
                    run_id, turn_id, item_id, payload_json, idempotency_key, redaction_state
             FROM events
             WHERE sequence > ?1
             ORDER BY sequence ASC
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![since_sequence, limit as i64], |row| {
            Ok(EventRecord {
                sequence: row.get(0)?,
                event_id: row.get(1)?,
                kind: row.get(2)?,
                actor: row.get(3)?,
                project_id: optional_id(row.get::<_, Option<String>>(4)?),
                task_id: optional_id(row.get::<_, Option<String>>(5)?),
                agent_id: optional_id(row.get::<_, Option<String>>(6)?),
                session_id: optional_id(row.get::<_, Option<String>>(7)?),
                run_id: optional_id(row.get::<_, Option<String>>(8)?),
                turn_id: row.get(9)?,
                item_id: row.get(10)?,
                payload_json: row.get(11)?,
                idempotency_key: row.get(12)?,
                redaction_state: row.get(13)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    /// Events strictly after `since_sequence` for one session, in ascending
    /// sequence order (ST4, session-scoped tail).
    ///
    /// Same forward-read semantics as [`Self::events_after`] but filtered to a
    /// single session, so a `Subscribe { session_id, from_sequence }` that names
    /// a session catches up on only that session's committed events.
    pub fn events_after_for_session(
        &self,
        session_id: &SessionId,
        since_sequence: i64,
        limit: usize,
    ) -> StateResult<Vec<EventRecord>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT sequence, event_id, kind, actor, project_id, task_id, agent_id, session_id,
                    run_id, turn_id, item_id, payload_json, idempotency_key, redaction_state
             FROM events
             WHERE sequence > ?1 AND session_id = ?2
             ORDER BY sequence ASC
             LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![since_sequence, session_id.as_str(), limit as i64],
            |row| {
                Ok(EventRecord {
                    sequence: row.get(0)?,
                    event_id: row.get(1)?,
                    kind: row.get(2)?,
                    actor: row.get(3)?,
                    project_id: optional_id(row.get::<_, Option<String>>(4)?),
                    task_id: optional_id(row.get::<_, Option<String>>(5)?),
                    agent_id: optional_id(row.get::<_, Option<String>>(6)?),
                    session_id: optional_id(row.get::<_, Option<String>>(7)?),
                    run_id: optional_id(row.get::<_, Option<String>>(8)?),
                    turn_id: row.get(9)?,
                    item_id: row.get(10)?,
                    payload_json: row.get(11)?,
                    idempotency_key: row.get(12)?,
                    redaction_state: row.get(13)?,
                })
            },
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    /// All events for a single turn within a session, in ascending sequence
    /// order, with NO recency cap.
    ///
    /// Unlike [`Self::recent_events_for_session`] (which keeps only the most
    /// recent `limit` events for diagnostics), this query is turn-scoped and
    /// unbounded: a turn's full projected event set is the source the
    /// replay-stable `TurnFinished` is re-derived from, so it must never be
    /// truncated. A session with thousands of events across many turns still
    /// reconstructs an early turn from its complete event set.
    pub fn events_for_session_turn(
        &self,
        session_id: &SessionId,
        turn_id: &str,
    ) -> StateResult<Vec<EventRecord>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT sequence, event_id, kind, actor, project_id, task_id, agent_id, session_id,
                    run_id, turn_id, item_id, payload_json, idempotency_key, redaction_state
             FROM events
             WHERE session_id = ?1 AND turn_id = ?2
             ORDER BY sequence ASC",
        )?;
        let rows = statement.query_map(params![session_id.as_str(), turn_id], |row| {
            Ok(EventRecord {
                sequence: row.get(0)?,
                event_id: row.get(1)?,
                kind: row.get(2)?,
                actor: row.get(3)?,
                project_id: optional_id(row.get::<_, Option<String>>(4)?),
                task_id: optional_id(row.get::<_, Option<String>>(5)?),
                agent_id: optional_id(row.get::<_, Option<String>>(6)?),
                session_id: optional_id(row.get::<_, Option<String>>(7)?),
                run_id: optional_id(row.get::<_, Option<String>>(8)?),
                turn_id: row.get(9)?,
                item_id: row.get(10)?,
                payload_json: row.get(11)?,
                idempotency_key: row.get(12)?,
                redaction_state: row.get(13)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    /// All `evidence.recorded` events for a single run, in ascending sequence
    /// order, with NO recency cap.
    ///
    /// Unlike [`Self::recent_events_for_session`] (which keeps only the most
    /// recent `limit` events for the whole session, a UI/diagnostics tail), this
    /// read is run-scoped and unbounded: the SG7 run score is a deterministic
    /// function of a run's observed verification evidence, so every matching
    /// event for the run must be visible regardless of how many later events the
    /// long-lived session accumulated. The `run_id` and `kind` filters are
    /// applied in SQL (before any limit), so a session with thousands of events
    /// from other turns/runs never truncates an early verification verdict out
    /// of the scored set, keeping the score stable and idempotent across
    /// re-scores as the session grows.
    pub fn evidence_events_for_run(&self, run_id: &RunId) -> StateResult<Vec<EventRecord>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT sequence, event_id, kind, actor, project_id, task_id, agent_id, session_id,
                    run_id, turn_id, item_id, payload_json, idempotency_key, redaction_state
             FROM events
             WHERE run_id = ?1 AND kind = ?2
             ORDER BY sequence ASC",
        )?;
        let rows = statement.query_map(
            params![run_id.as_str(), EventKind::EvidenceRecorded.as_str()],
            |row| {
                Ok(EventRecord {
                    sequence: row.get(0)?,
                    event_id: row.get(1)?,
                    kind: row.get(2)?,
                    actor: row.get(3)?,
                    project_id: optional_id(row.get::<_, Option<String>>(4)?),
                    task_id: optional_id(row.get::<_, Option<String>>(5)?),
                    agent_id: optional_id(row.get::<_, Option<String>>(6)?),
                    session_id: optional_id(row.get::<_, Option<String>>(7)?),
                    run_id: optional_id(row.get::<_, Option<String>>(8)?),
                    turn_id: row.get(9)?,
                    item_id: row.get(10)?,
                    payload_json: row.get(11)?,
                    idempotency_key: row.get(12)?,
                    redaction_state: row.get(13)?,
                })
            },
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    /// Every event keyed by one of `item_ids` (in its `item_id` column), in
    /// ascending sequence order. This is the item-scoped read behind the goal
    /// timeline (GO5/GO10): a goal's lifecycle / report / requirement /
    /// continuation events are keyed by their domain id as `item_id`, so a goal
    /// timeline is a `WHERE item_id IN (...)` filter ordered by sequence -- it is
    /// NOT a bounded prefix scan of the whole project log, so a goal event past
    /// any global event count is still returned. Mirrors
    /// [`Self::evidence_events_for_run`].
    pub fn events_for_items(&self, item_ids: &[String]) -> StateResult<Vec<EventRecord>> {
        if item_ids.is_empty() {
            return Ok(Vec::new());
        }
        let connection = Connection::open(&self.db_path)?;
        let placeholders = std::iter::repeat_n("?", item_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT sequence, event_id, kind, actor, project_id, task_id, agent_id, session_id,
                    run_id, turn_id, item_id, payload_json, idempotency_key, redaction_state
             FROM events
             WHERE item_id IN ({placeholders})
             ORDER BY sequence ASC"
        );
        let mut statement = connection.prepare(&sql)?;
        let params = rusqlite::params_from_iter(item_ids.iter().map(String::as_str));
        let rows = statement.query_map(params, |row| {
            Ok(EventRecord {
                sequence: row.get(0)?,
                event_id: row.get(1)?,
                kind: row.get(2)?,
                actor: row.get(3)?,
                project_id: optional_id(row.get::<_, Option<String>>(4)?),
                task_id: optional_id(row.get::<_, Option<String>>(5)?),
                agent_id: optional_id(row.get::<_, Option<String>>(6)?),
                session_id: optional_id(row.get::<_, Option<String>>(7)?),
                run_id: optional_id(row.get::<_, Option<String>>(8)?),
                turn_id: row.get(9)?,
                item_id: row.get(10)?,
                payload_json: row.get(11)?,
                idempotency_key: row.get(12)?,
                redaction_state: row.get(13)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    /// The `turn_id` of the most recently appended event for a session that
    /// carries a (non-null) turn id, or `None` when the session has no
    /// turn-keyed events yet.
    ///
    /// This is the command-path turn resolver: an operator `interrupt`/`stop`
    /// command names an agent, not a turn, so the controller resolves the
    /// session's active turn from the persisted event log here and routes the
    /// terminal event through the turn-keyed path. Highest `sequence` wins, so
    /// the answer is the latest turn the session actually ran (the first
    /// `send_task` turn, or the latest `redirect` turn), making the persisted
    /// `session.interrupted`/`session.stopped` reconstructable by that turn.
    pub fn latest_turn_for_session(&self, session_id: &SessionId) -> StateResult<Option<String>> {
        let connection = Connection::open(&self.db_path)?;
        let turn_id = connection
            .query_row(
                "SELECT turn_id
                 FROM events
                 WHERE session_id = ?1 AND turn_id IS NOT NULL
                 ORDER BY sequence DESC
                 LIMIT 1",
                params![session_id.as_str()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?;
        Ok(turn_id.flatten())
    }

    /// Project the multi-turn conversation thread for a session, incrementally
    /// by sequence (ST5).
    ///
    /// This reads the session's committed events strictly after `since_sequence`
    /// (via the same [`Self::events_after_for_session`] forward read the ST4
    /// `Subscribe` backlog uses) and folds them into a [`SessionThread`] through
    /// the pure [`SessionThread::project`] projection. Because both the read and
    /// the fold are deterministic over the durable log, the thread rebuilds
    /// identically on restart/replay, and because the watermark semantics match
    /// `events_after_for_session`, a thread read composes gap-free and
    /// duplicate-free with a `Subscribe` resuming from the same `since_sequence`.
    ///
    /// `since_sequence` of `0` reads the full thread from the start of the log;
    /// a caller pages forward by advancing it to the returned
    /// [`SessionThread::next_sequence`]. `limit` bounds the catch-up read so a
    /// session with a very long log reads a bounded page.
    pub fn session_thread(
        &self,
        session_id: &SessionId,
        since_sequence: i64,
        limit: usize,
    ) -> StateResult<crate::SessionThread> {
        let events = self.events_after_for_session(session_id, since_sequence, limit)?;
        Ok(crate::SessionThread::project(
            session_id.clone(),
            since_sequence,
            &events,
        ))
    }

    /// GA1 (goal-orchestration GO1): read the goal lifecycle read model. The GA5
    /// auditor and GA4 scheduler operate on this rebuilt state rather than a live
    /// transcript, so this read must round-trip identically across a rebuild.
    pub fn goal(&self, goal_id: &GoalId) -> StateResult<Option<GoalProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let goal = connection
            .query_row(
                "SELECT goal_id, project_id, task_id, agent_id, session_id, parent_goal_id,
                        attempt_run_id, objective, status, success_criteria_json, constraints_json,
                        verification_surface_json, budget_json, stop_conditions_json,
                        blocker_reason, updated_sequence
                 FROM goals
                 WHERE goal_id = ?1",
                params![goal_id.as_str()],
                goal_from_row,
            )
            .optional()?;
        Ok(goal)
    }

    /// GA2 (goal-orchestration GO4/GO5): every goal owned by a project, newest
    /// activity first (highest `updated_sequence`), then by id for a stable order.
    /// This backs the operator `goals` read surface and the historical-report
    /// rebuild; like every goal read it round-trips identically across a rebuild.
    pub fn goals_for_project(&self, project_id: &ProjectId) -> StateResult<Vec<GoalProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT goal_id, project_id, task_id, agent_id, session_id, parent_goal_id,
                    attempt_run_id, objective, status, success_criteria_json, constraints_json,
                    verification_surface_json, budget_json, stop_conditions_json,
                    blocker_reason, updated_sequence
             FROM goals
             WHERE project_id = ?1
             ORDER BY updated_sequence DESC, goal_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], goal_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    /// GA1 (goal-orchestration GO3): the per-requirement status ledger for a goal.
    pub fn requirement_ledgers_for_goal(
        &self,
        goal_id: &GoalId,
    ) -> StateResult<Vec<RequirementLedgerProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT requirement_id, goal_id, project_id, summary, status, last_status_source,
                    updated_sequence
             FROM requirement_ledgers
             WHERE goal_id = ?1
             ORDER BY requirement_id ASC",
        )?;
        let rows = statement.query_map(params![goal_id.as_str()], requirement_ledger_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    /// GA1 (goal-orchestration GO3): the agent-report / story ledger for a goal,
    /// oldest first, preserving the observed-vs-reported `source` tag.
    pub fn goal_reports_for_goal(
        &self,
        goal_id: &GoalId,
    ) -> StateResult<Vec<GoalReportProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT goal_report_id, goal_id, project_id, session_id, requirement_id, report_kind,
                    source, confidence, summary, body_artifact_id, evidence_id, updated_sequence
             FROM goal_reports
             WHERE goal_id = ?1
             ORDER BY updated_sequence ASC, goal_report_id ASC",
        )?;
        let rows = statement.query_map(params![goal_id.as_str()], goal_report_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    /// GA1 (goal-orchestration GO3/GO8): the recorded continuation decisions for a
    /// goal, oldest first.
    pub fn goal_continuations_for_goal(
        &self,
        goal_id: &GoalId,
    ) -> StateResult<Vec<GoalContinuationProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT continuation_id, goal_id, project_id, attempt_run_id, decision, reason,
                    updated_sequence
             FROM goal_continuations
             WHERE goal_id = ?1
             ORDER BY updated_sequence ASC, continuation_id ASC",
        )?;
        let rows = statement.query_map(params![goal_id.as_str()], goal_continuation_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    /// GA1 (goal-orchestration GO12): the observed delegated-provider goal records
    /// for a goal, oldest first. These are observed-not-authoritative evidence.
    pub fn delegated_provider_goals_for_goal(
        &self,
        goal_id: &GoalId,
    ) -> StateResult<Vec<DelegatedProviderGoalProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT delegated_goal_id, goal_id, project_id, session_id, provider_kind,
                    provider_goal_ref, provider_state, source, body_artifact_id, updated_sequence
             FROM delegated_provider_goals
             WHERE goal_id = ?1
             ORDER BY updated_sequence ASC, delegated_goal_id ASC",
        )?;
        let rows =
            statement.query_map(params![goal_id.as_str()], delegated_provider_goal_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    /// GA3 (goal-orchestration GO7): the artifact record for `artifact_id`, or
    /// `None` if absent.
    ///
    /// The continuation context packet references a report/evidence body by its
    /// artifact id and carries the artifact's `content_hash` and `redaction_state`
    /// for provenance, WITHOUT ever materializing the body. This read returns just
    /// that metadata so the packet stays bounded and explainable: a referenced
    /// fragment names its source and content hash; it never inlines raw content.
    pub fn artifact_by_id(&self, artifact_id: &str) -> StateResult<Option<ArtifactRecord>> {
        let connection = Connection::open(&self.db_path)?;
        let artifact = connection
            .query_row(
                "SELECT artifact_id, project_id, session_id, run_id, kind, uri, content_hash,
                        size_bytes, redaction_state
                 FROM artifacts
                 WHERE artifact_id = ?1",
                params![artifact_id],
                |row| {
                    Ok(ArtifactRecord {
                        artifact_id: row.get(0)?,
                        project_id: optional_id(row.get::<_, Option<String>>(1)?),
                        session_id: optional_id(row.get::<_, Option<String>>(2)?),
                        run_id: optional_id(row.get::<_, Option<String>>(3)?),
                        kind: row.get(4)?,
                        uri: row.get(5)?,
                        content_hash: row.get(6)?,
                        size_bytes: row.get(7)?,
                        redaction_state: RedactionState::from_wire(&row.get::<_, String>(8)?)
                            .unwrap_or(RedactionState::Unknown),
                    })
                },
            )
            .optional()?;
        Ok(artifact)
    }
}

/// Extract `(external_pid, runtime_process_ref)` from a `run.started` payload
/// if it carries the in-flight marker (RTL10). Returns `None` when the payload
/// is not the in-flight marker shape.
/// The fields the in-flight `run.started` marker carries that restart recovery
/// needs to probe and reap the persisted process group (RTL10).
struct InFlightMarker {
    external_pid: Option<u32>,
    boot_id: Option<String>,
    runtime_process_ref: Option<String>,
}

fn parse_inflight_marker(payload_json: &str) -> Option<InFlightMarker> {
    let value: serde_json::Value = serde_json::from_str(payload_json).ok()?;
    let object = value.as_object()?;
    let external_pid = object
        .get("external_pid")
        .and_then(|pid| pid.as_u64())
        .and_then(|pid| u32::try_from(pid).ok())
        // A zero PID is not a real process group target (it would be the
        // caller's own group); treat it as "no process to reap".
        .filter(|pid| *pid != 0);
    let boot_id = object
        .get("boot_id")
        .and_then(|boot| boot.as_str())
        .map(ToString::to_string);
    let runtime_process_ref = object
        .get("runtime_process_ref")
        .and_then(|reference| reference.as_str())
        .map(ToString::to_string);
    if external_pid.is_none() && runtime_process_ref.is_none() {
        return None;
    }
    Some(InFlightMarker {
        external_pid,
        boot_id,
        runtime_process_ref,
    })
}

fn run_score_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunScoreProjection> {
    Ok(RunScoreProjection {
        run_score_id: row.get(0)?,
        project_id: ProjectId::new(row.get::<_, String>(1)?),
        task_id: optional_id(row.get::<_, Option<String>>(2)?),
        session_id: SessionId::new(row.get::<_, String>(3)?),
        run_id: RunId::new(row.get::<_, String>(4)?),
        outcome: row.get(5)?,
        passed: row.get::<_, i64>(6)? != 0,
        criteria_total: row.get(7)?,
        criteria_met: row.get(8)?,
        observed_evidence_count: row.get(9)?,
        started_at: row.get(10)?,
        completed_at: row.get(11)?,
        duration_millis: row.get(12)?,
        score_inputs_json: row.get(13)?,
        updated_sequence: row.get(14)?,
    })
}

fn checkpoint_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CheckpointProjection> {
    Ok(CheckpointProjection {
        checkpoint_id: row.get(0)?,
        project_id: ProjectId::new(row.get::<_, String>(1)?),
        session_id: SessionId::new(row.get::<_, String>(2)?),
        run_id: RunId::new(row.get::<_, String>(3)?),
        turn_id: row.get(4)?,
        kind: row.get(5)?,
        commit_ref: row.get(6)?,
        workspace_root: row.get(7)?,
        shadow_git_dir: row.get(8)?,
        content_hash: row.get(9)?,
        created_at: row.get(10)?,
        restored_at: row.get(11)?,
        updated_sequence: row.get(12)?,
    })
}

fn source_binding_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SourceBindingProjection> {
    Ok(SourceBindingProjection {
        source_binding_id: row.get(0)?,
        project_id: ProjectId::new(row.get::<_, String>(1)?),
        task_id: TaskId::new(row.get::<_, String>(2)?),
        source_kind: row.get(3)?,
        source_task_id: row.get(4)?,
        source_path: row.get(5)?,
        source_anchor: row.get(6)?,
        source_hash: row.get(7)?,
        binding_status: row.get(8)?,
        updated_sequence: row.get(9)?,
    })
}

fn goal_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GoalProjection> {
    Ok(GoalProjection {
        goal_id: GoalId::new(row.get::<_, String>(0)?),
        project_id: ProjectId::new(row.get::<_, String>(1)?),
        task_id: optional_id(row.get::<_, Option<String>>(2)?),
        agent_id: optional_id(row.get::<_, Option<String>>(3)?),
        session_id: optional_id(row.get::<_, Option<String>>(4)?),
        parent_goal_id: optional_id(row.get::<_, Option<String>>(5)?),
        attempt_run_id: optional_id(row.get::<_, Option<String>>(6)?),
        objective: row.get(7)?,
        status: row.get(8)?,
        success_criteria_json: row.get(9)?,
        constraints_json: row.get(10)?,
        verification_surface_json: row.get(11)?,
        budget_json: row.get(12)?,
        stop_conditions_json: row.get(13)?,
        blocker_reason: row.get(14)?,
        updated_sequence: row.get(15)?,
    })
}

fn requirement_ledger_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<RequirementLedgerProjection> {
    Ok(RequirementLedgerProjection {
        requirement_id: RequirementId::new(row.get::<_, String>(0)?),
        goal_id: GoalId::new(row.get::<_, String>(1)?),
        project_id: ProjectId::new(row.get::<_, String>(2)?),
        summary: row.get(3)?,
        status: row.get(4)?,
        last_status_source: row.get(5)?,
        updated_sequence: row.get(6)?,
    })
}

fn goal_report_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GoalReportProjection> {
    Ok(GoalReportProjection {
        goal_report_id: row.get(0)?,
        goal_id: GoalId::new(row.get::<_, String>(1)?),
        project_id: ProjectId::new(row.get::<_, String>(2)?),
        session_id: optional_id(row.get::<_, Option<String>>(3)?),
        requirement_id: optional_id(row.get::<_, Option<String>>(4)?),
        report_kind: row.get(5)?,
        source: row.get(6)?,
        confidence: row.get(7)?,
        summary: row.get(8)?,
        body_artifact_id: row.get(9)?,
        evidence_id: optional_id(row.get::<_, Option<String>>(10)?),
        updated_sequence: row.get(11)?,
    })
}

fn goal_continuation_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<GoalContinuationProjection> {
    Ok(GoalContinuationProjection {
        continuation_id: row.get(0)?,
        goal_id: GoalId::new(row.get::<_, String>(1)?),
        project_id: ProjectId::new(row.get::<_, String>(2)?),
        attempt_run_id: optional_id(row.get::<_, Option<String>>(3)?),
        decision: row.get(4)?,
        reason: row.get(5)?,
        updated_sequence: row.get(6)?,
    })
}

fn delegated_provider_goal_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<DelegatedProviderGoalProjection> {
    Ok(DelegatedProviderGoalProjection {
        delegated_goal_id: row.get(0)?,
        goal_id: GoalId::new(row.get::<_, String>(1)?),
        project_id: ProjectId::new(row.get::<_, String>(2)?),
        session_id: optional_id(row.get::<_, Option<String>>(3)?),
        provider_kind: row.get(4)?,
        provider_goal_ref: row.get(5)?,
        provider_state: row.get(6)?,
        source: row.get(7)?,
        body_artifact_id: row.get(8)?,
        updated_sequence: row.get(9)?,
    })
}
