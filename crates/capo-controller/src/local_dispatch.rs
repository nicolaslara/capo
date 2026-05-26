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
}
