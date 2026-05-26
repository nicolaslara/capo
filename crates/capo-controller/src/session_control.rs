use super::*;

impl FakeBoundaryController {
    pub fn refs_for_agent_name(&self, agent_name: &str) -> StateResult<FakeRunRefs> {
        let agent = self
            .state
            .agent_by_name(agent_name)?
            .ok_or_else(|| missing_read_model("agent.name", &agent_name))?;
        let session_id = agent
            .current_session_id
            .clone()
            .ok_or_else(|| missing_read_model("agent.current_session_id", &agent.agent_id))?;
        let session = self
            .state
            .session(&session_id)?
            .ok_or_else(|| missing_read_model("session", &session_id))?;
        let run = self
            .state
            .run_for_session(&session_id)?
            .ok_or_else(|| missing_read_model("run.session_id", &session_id))?;
        Ok(FakeRunRefs {
            task_id: session
                .task_id
                .ok_or_else(|| missing_read_model("session.task_id", &session_id))?,
            agent_id: agent.agent_id,
            session_id,
            run_id: run.run_id,
            runtime_process_ref: format!("fake-runtime-process-{agent_name}"),
            external_session_ref: format!("fake-adapter-session-{agent_name}"),
        })
    }

    pub fn observe_agent_name(&self, agent_name: &str) -> StateResult<FakeReadModelObservation> {
        let refs = self.refs_for_agent_name(agent_name)?;
        self.observe(&refs)
    }

    pub fn redirect_agent_name(
        &self,
        agent_name: &str,
        goal: &str,
    ) -> StateResult<FakeReadModelObservation> {
        let registration = self.registration_for_agent_name(agent_name)?;
        let refs = self.refs_for_agent_name(agent_name)?;
        self.redirect(&registration, &refs, goal)
    }

    pub fn redirect(
        &self,
        registration: &FakeAgentRegistration,
        refs: &FakeRunRefs,
        goal: &str,
    ) -> StateResult<FakeReadModelObservation> {
        let session = self
            .state
            .session(&refs.session_id)?
            .ok_or_else(|| missing_read_model("session", &refs.session_id))?;
        let task = self
            .state
            .task(&refs.task_id)?
            .ok_or_else(|| missing_read_model("task", &refs.task_id))?;
        let adapter_session = self
            .adapter
            .attach_session(refs.session_id.clone(), refs.external_session_ref.clone());
        let turn_id = TurnId::new(format!("redirect-{}", refs.session_id));
        let adapter_output = self.adapter.send_turn(
            &adapter_session,
            FakeAdapterTurnRequest {
                turn_id: turn_id.clone(),
                agent_name: registration.agent_name.clone(),
                goal: goal.to_string(),
            },
        );

        self.state.append_event(
            scoped_event(
                &format!(
                    "event-session-redirected-{}-{}",
                    refs.session_id,
                    stable_hash(goal.as_bytes())
                ),
                EventKind::SessionRedirected,
                &self.project_id,
                &refs.task_id,
                &registration.agent_id,
                &refs.session_id,
                &refs.run_id,
            )
            .with_turn(format!("{turn_id}-{}", stable_hash(goal.as_bytes())))
            .with_payload(format!(
                "{{\"goal\":\"{}\",\"adapter_summary\":\"{}\"}}",
                escape_json(goal),
                escape_json(&adapter_output.summary)
            )),
            &[
                ProjectionRecord::Task(TaskProjection {
                    task_id: refs.task_id.clone(),
                    project_id: self.project_id.clone(),
                    title: session.title.clone(),
                    capo_execution_status: "active".to_string(),
                    active_session_id: Some(refs.session_id.clone()),
                    latest_summary: Some(adapter_output.summary.clone()),
                    evidence_id: task.evidence_id,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Agent(AgentProjection {
                    agent_id: refs.agent_id.clone(),
                    project_id: self.project_id.clone(),
                    name: registration.agent_name.clone(),
                    status: "running".to_string(),
                    current_session_id: Some(refs.session_id.clone()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Session(SessionProjection {
                    session_id: refs.session_id.clone(),
                    project_id: self.project_id.clone(),
                    task_id: Some(refs.task_id.clone()),
                    agent_id: refs.agent_id.clone(),
                    title: session.title,
                    status: adapter_output.status,
                    current_goal: goal.to_string(),
                    latest_summary: Some(adapter_output.summary),
                    latest_confidence: Some(78),
                    latest_blocker: None,
                    updated_sequence: 0,
                }),
            ],
        )?;

        self.observe(refs)
    }

    pub fn interrupt_agent_name(
        &self,
        agent_name: &str,
        reason: &str,
    ) -> StateResult<FakeReadModelObservation> {
        let registration = self.registration_for_agent_name(agent_name)?;
        let refs = self.refs_for_agent_name(agent_name)?;
        self.interrupt(&registration, &refs, reason)
    }

    pub fn stop_agent_name(
        &self,
        agent_name: &str,
        reason: &str,
    ) -> StateResult<FakeReadModelObservation> {
        let registration = self.registration_for_agent_name(agent_name)?;
        let refs = self.refs_for_agent_name(agent_name)?;
        self.stop(&registration, &refs, reason)
    }

    pub fn interrupt(
        &self,
        registration: &FakeAgentRegistration,
        refs: &FakeRunRefs,
        reason: &str,
    ) -> StateResult<FakeReadModelObservation> {
        let session = self
            .state
            .session(&refs.session_id)?
            .ok_or_else(|| missing_read_model("session", &refs.session_id))?;
        let task = self
            .state
            .task(&refs.task_id)?
            .ok_or_else(|| missing_read_model("task", &refs.task_id))?;
        let runtime_process = self
            .runtime
            .attach_process(refs.run_id.clone(), refs.runtime_process_ref.clone());
        let interrupted_process = self.runtime.interrupt(&runtime_process, reason);
        let adapter_session = self
            .adapter
            .attach_session(refs.session_id.clone(), refs.external_session_ref.clone());
        let adapter_output = self.adapter.interrupt(&adapter_session, reason);

        self.state.append_event(
            scoped_event(
                &format!("event-session-interrupted-{}", refs.session_id),
                EventKind::SessionInterrupted,
                &self.project_id,
                &refs.task_id,
                &registration.agent_id,
                &refs.session_id,
                &refs.run_id,
            )
            .with_payload(format!(
                "{{\"reason\":\"{}\",\"adapter_summary\":\"{}\"}}",
                escape_json(reason),
                escape_json(&adapter_output.summary)
            )),
            &[
                ProjectionRecord::Task(TaskProjection {
                    task_id: refs.task_id.clone(),
                    project_id: self.project_id.clone(),
                    title: session.title.clone(),
                    capo_execution_status: "canceled".to_string(),
                    active_session_id: Some(refs.session_id.clone()),
                    latest_summary: Some(adapter_output.summary),
                    evidence_id: task.evidence_id,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Agent(AgentProjection {
                    agent_id: refs.agent_id.clone(),
                    project_id: self.project_id.clone(),
                    name: registration.agent_name.clone(),
                    status: "available".to_string(),
                    current_session_id: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Session(SessionProjection {
                    session_id: refs.session_id.clone(),
                    project_id: self.project_id.clone(),
                    task_id: Some(refs.task_id.clone()),
                    agent_id: refs.agent_id.clone(),
                    title: session.title,
                    status: "canceled".to_string(),
                    current_goal: session.current_goal,
                    latest_summary: Some(format!("Interrupted: {reason}")),
                    latest_confidence: Some(70),
                    latest_blocker: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id: refs.run_id.clone(),
                    session_id: refs.session_id.clone(),
                    status: interrupted_process.status,
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        )?;

        self.observe(refs)
    }

    pub fn stop(
        &self,
        registration: &FakeAgentRegistration,
        refs: &FakeRunRefs,
        reason: &str,
    ) -> StateResult<FakeReadModelObservation> {
        let session = self
            .state
            .session(&refs.session_id)?
            .ok_or_else(|| missing_read_model("session", &refs.session_id))?;
        let task = self
            .state
            .task(&refs.task_id)?
            .ok_or_else(|| missing_read_model("task", &refs.task_id))?;
        let runtime_process = self
            .runtime
            .attach_process(refs.run_id.clone(), refs.runtime_process_ref.clone());
        let stopped_process = self.runtime.stop(&runtime_process, reason);
        let adapter_session = self
            .adapter
            .attach_session(refs.session_id.clone(), refs.external_session_ref.clone());
        let adapter_output = self.adapter.stop(&adapter_session, reason);

        self.state.append_event(
            scoped_event(
                &format!("event-session-stopped-{}", refs.session_id),
                EventKind::SessionStopped,
                &self.project_id,
                &refs.task_id,
                &registration.agent_id,
                &refs.session_id,
                &refs.run_id,
            )
            .with_payload(format!(
                "{{\"reason\":\"{}\",\"adapter_summary\":\"{}\"}}",
                escape_json(reason),
                escape_json(&adapter_output.summary)
            )),
            &[
                ProjectionRecord::Task(TaskProjection {
                    task_id: refs.task_id.clone(),
                    project_id: self.project_id.clone(),
                    title: session.title.clone(),
                    capo_execution_status: "completed".to_string(),
                    active_session_id: Some(refs.session_id.clone()),
                    latest_summary: Some(adapter_output.summary),
                    evidence_id: task.evidence_id,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Agent(AgentProjection {
                    agent_id: refs.agent_id.clone(),
                    project_id: self.project_id.clone(),
                    name: registration.agent_name.clone(),
                    status: "available".to_string(),
                    current_session_id: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Session(SessionProjection {
                    session_id: refs.session_id.clone(),
                    project_id: self.project_id.clone(),
                    task_id: Some(refs.task_id.clone()),
                    agent_id: refs.agent_id.clone(),
                    title: session.title,
                    status: "completed".to_string(),
                    current_goal: session.current_goal,
                    latest_summary: Some(format!("Stopped: {reason}")),
                    latest_confidence: Some(70),
                    latest_blocker: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id: refs.run_id.clone(),
                    session_id: refs.session_id.clone(),
                    status: stopped_process.status,
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        )?;

        self.observe(refs)
    }

    pub fn observe(&self, refs: &FakeRunRefs) -> StateResult<FakeReadModelObservation> {
        Ok(FakeReadModelObservation {
            task: self
                .state
                .task(&refs.task_id)?
                .ok_or_else(|| missing_read_model("task", &refs.task_id))?,
            agent: self
                .state
                .agent(&refs.agent_id)?
                .ok_or_else(|| missing_read_model("agent", &refs.agent_id))?,
            session: self
                .state
                .session(&refs.session_id)?
                .ok_or_else(|| missing_read_model("session", &refs.session_id))?,
            run: self
                .state
                .run(&refs.run_id)?
                .ok_or_else(|| missing_read_model("run", &refs.run_id))?,
            recent_events: self.state.recent_events_for_session(&refs.session_id, 16)?,
        })
    }
}
