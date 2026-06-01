use super::*;

impl FakeBoundaryController {
    pub fn plan_local_adapter_dispatch(
        &self,
        adapter: &str,
        agent_name: &str,
        goal: &str,
        workspace_root: PathBuf,
        artifact_root: PathBuf,
    ) -> Result<LocalAdapterDispatchPlan, String> {
        let registration = self
            .registration_for_agent_name(agent_name)
            .map_err(|error| format!("{error:?}"))?;
        let session_id = SessionId::new(format!("session-{}", registration.agent_name));
        let run_id = RunId::new(format!("run-{}", registration.agent_name));
        let launch_plan = match adapter {
            "codex" | "codex-exec" | "codex_exec" => {
                CodexExecAdapter::local_launch_plan(workspace_root, artifact_root, goal)
            }
            "claude" | "claude-code" | "claude_code" => {
                ClaudeCodeAdapter::local_launch_plan(workspace_root, artifact_root, goal)
            }
            other => {
                return Err(format!(
                    "unsupported local adapter dispatch plan: {other}; expected codex or claude"
                ));
            }
        };
        launch_plan.assert_subscription_safe()?;
        let runtime_request = launch_plan.runtime_request(run_id.clone());
        Ok(LocalAdapterDispatchPlan {
            project_id: self.project_id.clone(),
            agent_id: registration.agent_id,
            agent_name: registration.agent_name,
            session_id,
            run_id,
            launch_plan,
            runtime_program: runtime_request.program,
            runtime_arg_count: runtime_request.argv.len(),
            runtime_cwd: runtime_request.cwd,
            request_env_count: runtime_request.env.len(),
        })
    }

    pub fn prepare_local_adapter_dispatch_run(
        &self,
        start: LocalAdapterDispatchRunStart,
    ) -> StateResult<FakeRunRefs> {
        let registration = self.registration_for_agent_name(&start.agent_name)?;
        self.state.append_event(
            scoped_event(
                &format!("event-local-adapter-dispatch-started-{}", start.session_id),
                EventKind::SessionStarted,
                &self.project_id,
                &start.task_id,
                &registration.agent_id,
                &start.session_id,
                &start.run_id,
            )
            .with_payload(format!(
                "{{\"goal\":\"{}\",\"runtime_process_ref\":\"{}\",\"external_session_ref\":\"{}\",\"provider_cli_executed\":{},\"adapter_kind\":\"{}\"}}",
                escape_json(&start.goal),
                escape_json(&start.runtime_process_ref),
                escape_json(&start.external_session_ref),
                start.provider_cli_executed,
                escape_json(&start.adapter_kind)
            )),
            &[
                ProjectionRecord::Task(TaskProjection {
                    task_id: start.task_id.clone(),
                    project_id: self.project_id.clone(),
                    title: start.goal.clone(),
                    capo_execution_status: "active".to_string(),
                    active_session_id: Some(start.session_id.clone()),
                    latest_summary: None,
                    evidence_id: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Agent(AgentProjection {
                    agent_id: registration.agent_id.clone(),
                    project_id: self.project_id.clone(),
                    name: registration.agent_name.clone(),
                    status: "running".to_string(),
                    current_session_id: Some(start.session_id.clone()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Session(SessionProjection {
                    session_id: start.session_id.clone(),
                    project_id: self.project_id.clone(),
                    task_id: Some(start.task_id.clone()),
                    agent_id: registration.agent_id.clone(),
                    title: start.goal.clone(),
                    status: "active".to_string(),
                    current_goal: start.goal.clone(),
                    latest_summary: None,
                    latest_confidence: None,
                    latest_blocker: None,
                    external_session_ref: Some(start.external_session_ref.clone()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id: start.run_id.clone(),
                    session_id: start.session_id.clone(),
                    status: "running".to_string(),
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        )?;
        Ok(FakeRunRefs {
            task_id: start.task_id,
            agent_id: registration.agent_id,
            session_id: start.session_id,
            run_id: start.run_id,
            runtime_process_ref: start.runtime_process_ref,
            external_session_ref: start.external_session_ref,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalAdapterDispatchRunStart {
    pub agent_name: String,
    pub task_id: TaskId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub goal: String,
    pub runtime_process_ref: String,
    pub external_session_ref: String,
    pub provider_cli_executed: bool,
    pub adapter_kind: String,
}
