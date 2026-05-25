//! Core domain vocabulary and persistence-free controller skeleton for Capo.
//!
//! This crate owns shared names and records. Concrete adapters, runtimes,
//! stores, tools, memory backends, and evaluators live in their boundary crates
//! and report their static-dispatch binding back to these core types.

use std::fmt;

/// Product name used by CLI/help surfaces.
pub const PRODUCT_NAME: &str = "Capo";

/// Boundary crates that make up the first prototype scaffold.
pub const BOUNDARY_CRATES: &[&str] = &[
    "capo-core",
    "capo-state",
    "capo-adapters",
    "capo-runtime",
    "capo-controller",
    "capo-tools",
    "capo-memory",
    "capo-query",
    "capo-eval",
    "capo-voice",
    "capo-workpads",
];

macro_rules! typed_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                let value = value.into();
                assert!(!value.trim().is_empty(), "typed IDs cannot be empty");
                Self(value)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

typed_id!(ProjectId);
typed_id!(TaskId);
typed_id!(AgentId);
typed_id!(SessionId);
typed_id!(RunId);
typed_id!(TurnId);
typed_id!(ToolCallId);
typed_id!(MemoryPacketId);
typed_id!(EvidenceId);
typed_id!(CommandId);
typed_id!(CapabilityProfileId);
typed_id!(ArtifactId);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputOrigin {
    Cli,
    Tui,
    Dashboard,
    Mobile,
    Voice,
    Api,
    System,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandTarget {
    Project(ProjectId),
    Task(TaskId),
    Agent(AgentId),
    Session(SessionId),
    Run(RunId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandIntent {
    InitializeProject,
    RegisterAgent,
    StartSession,
    SendTask,
    QueryStatus,
    RedirectSession,
    InterruptSession,
    Recover,
    ExportEvidence,
    IndexWorkpads,
    ImportWorkpadTask,
    WriteWorkpadProposal,
    ApplyWorkpadProposal,
    QueuePermissionApproval,
    DecidePermissionApproval,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandEnvelope {
    pub command_id: CommandId,
    pub origin: InputOrigin,
    pub actor_id: String,
    pub project_id: ProjectId,
    pub target: CommandTarget,
    pub intent: CommandIntent,
    pub text: Option<String>,
    pub structured_args: Vec<(String, String)>,
    pub attachments: Vec<ArtifactId>,
    pub risk: RiskLevel,
    pub idempotency_key: String,
}

impl CommandEnvelope {
    pub fn new(
        command_id: CommandId,
        origin: InputOrigin,
        actor_id: impl Into<String>,
        project_id: ProjectId,
        target: CommandTarget,
        intent: CommandIntent,
    ) -> Self {
        let actor_id = actor_id.into();
        let idempotency_key = format!("{origin:?}:{command_id}");

        Self {
            command_id,
            origin,
            actor_id,
            project_id,
            target,
            intent,
            text: None,
            structured_args: Vec::new(),
            attachments: Vec::new(),
            risk: RiskLevel::Low,
            idempotency_key,
        }
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskStatus {
    Pending,
    Active,
    Blocked,
    Completed,
    Failed,
    Canceled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionStatus {
    Starting,
    Active,
    WaitingForInput,
    WaitingForPermission,
    Canceling,
    Completed,
    Failed,
    Canceled,
    Recovering,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunStatus {
    Starting,
    Running,
    Stopping,
    Exited,
    Failed,
    Orphaned,
    Recovered,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TurnStatus {
    Open,
    Streaming,
    WaitingForTool,
    WaitingForPermission,
    Completed,
    Failed,
    Canceled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolCallStatus {
    Requested,
    Running,
    Completed,
    Failed,
    Canceled,
    ObservedOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewState {
    Draft,
    Generated,
    Reviewed,
    Promoted,
    Invalidated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Project {
    pub project_id: ProjectId,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Task {
    pub task_id: TaskId,
    pub project_id: ProjectId,
    pub title: String,
    pub status: TaskStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Agent {
    pub agent_id: AgentId,
    pub project_id: ProjectId,
    pub name: String,
    pub capability_profile_id: CapabilityProfileId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Session {
    pub session_id: SessionId,
    pub project_id: ProjectId,
    pub task_id: Option<TaskId>,
    pub agent_id: AgentId,
    pub title: String,
    pub status: SessionStatus,
    pub current_goal: String,
    pub latest_confidence: Option<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Run {
    pub run_id: RunId,
    pub session_id: SessionId,
    pub status: RunStatus,
    pub recovery_of_run_id: Option<RunId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Turn {
    pub turn_id: TurnId,
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub status: TurnStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolCall {
    pub tool_call_id: ToolCallId,
    pub session_id: SessionId,
    pub turn_id: Option<TurnId>,
    pub tool_name: String,
    pub status: ToolCallStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryPacket {
    pub memory_packet_id: MemoryPacketId,
    pub session_id: SessionId,
    pub purpose: String,
    pub review_state: ReviewState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Evidence {
    pub evidence_id: EvidenceId,
    pub project_id: ProjectId,
    pub session_id: Option<SessionId>,
    pub confidence: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityProfile {
    pub capability_profile_id: CapabilityProfileId,
    pub name: String,
    pub risk: RiskLevel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryKind {
    AgentAdapter,
    RuntimeRunner,
    ConnectivityTunnel,
    ProviderConnector,
    PermissionPolicy,
    ToolExposure,
    StateStore,
    MemoryBackend,
    EvaluationLayer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoundaryBinding {
    pub kind: BoundaryKind,
    pub variant: &'static str,
    pub fake: bool,
}

impl BoundaryBinding {
    pub const fn fake(kind: BoundaryKind, variant: &'static str) -> Self {
        Self {
            kind,
            variant,
            fake: true,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BoundarySet {
    bindings: Vec<BoundaryBinding>,
}

impl BoundarySet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, binding: BoundaryBinding) {
        self.bindings
            .retain(|existing| existing.kind != binding.kind);
        self.bindings.push(binding);
    }

    pub fn has(&self, kind: BoundaryKind) -> bool {
        self.bindings.iter().any(|binding| binding.kind == kind)
    }

    pub fn all_fake(&self) -> bool {
        self.bindings.iter().all(|binding| binding.fake)
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControllerPreview {
    pub accepted_command_id: CommandId,
    pub session: Session,
    pub run: Run,
    pub turn: Turn,
    pub boundary_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControllerError {
    MissingBoundary(BoundaryKind),
    UnsupportedIntent(CommandIntent),
    TargetIsNotAgent,
}

pub struct CapoController;

impl CapoController {
    pub fn new() -> Self {
        Self
    }

    pub fn preview_start_session(
        &self,
        command: &CommandEnvelope,
        boundaries: &BoundarySet,
    ) -> Result<ControllerPreview, ControllerError> {
        for kind in REQUIRED_FAKE_E2E_BOUNDARIES {
            if !boundaries.has(*kind) {
                return Err(ControllerError::MissingBoundary(*kind));
            }
        }

        if command.intent != CommandIntent::StartSession {
            return Err(ControllerError::UnsupportedIntent(command.intent));
        }

        let CommandTarget::Agent(agent_id) = &command.target else {
            return Err(ControllerError::TargetIsNotAgent);
        };

        let session = Session {
            session_id: SessionId::new(format!("session-{}", command.command_id)),
            project_id: command.project_id.clone(),
            task_id: None,
            agent_id: agent_id.clone(),
            title: "prototype fake session".to_string(),
            status: SessionStatus::Starting,
            current_goal: command.text.clone().unwrap_or_default(),
            latest_confidence: None,
        };
        let run = Run {
            run_id: RunId::new(format!("run-{}", command.command_id)),
            session_id: session.session_id.clone(),
            status: RunStatus::Starting,
            recovery_of_run_id: None,
        };
        let turn = Turn {
            turn_id: TurnId::new(format!("turn-{}", command.command_id)),
            session_id: session.session_id.clone(),
            run_id: Some(run.run_id.clone()),
            status: TurnStatus::Open,
        };

        Ok(ControllerPreview {
            accepted_command_id: command.command_id.clone(),
            session,
            run,
            turn,
            boundary_count: boundaries.len(),
        })
    }
}

impl Default for CapoController {
    fn default() -> Self {
        Self::new()
    }
}

pub const REQUIRED_FAKE_E2E_BOUNDARIES: &[BoundaryKind] = &[
    BoundaryKind::AgentAdapter,
    BoundaryKind::RuntimeRunner,
    BoundaryKind::ConnectivityTunnel,
    BoundaryKind::ProviderConnector,
    BoundaryKind::PermissionPolicy,
    BoundaryKind::ToolExposure,
    BoundaryKind::StateStore,
    BoundaryKind::MemoryBackend,
    BoundaryKind::EvaluationLayer,
];

/// Returns a stable, human-readable scaffold summary.
pub fn scaffold_summary() -> String {
    format!(
        "{PRODUCT_NAME} prototype scaffold: {} boundary crates",
        BOUNDARY_CRATES.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffold_summary_names_product() {
        assert!(scaffold_summary().contains(PRODUCT_NAME));
    }

    #[test]
    fn boundary_crates_include_core_boundaries() {
        assert!(BOUNDARY_CRATES.contains(&"capo-state"));
        assert!(BOUNDARY_CRATES.contains(&"capo-adapters"));
        assert!(BOUNDARY_CRATES.contains(&"capo-runtime"));
        assert!(BOUNDARY_CRATES.contains(&"capo-tools"));
        assert!(BOUNDARY_CRATES.contains(&"capo-memory"));
    }

    #[test]
    fn controller_rejects_missing_boundaries() {
        let command = start_session_command();
        let error = CapoController::new()
            .preview_start_session(&command, &BoundarySet::new())
            .unwrap_err();

        assert_eq!(
            error,
            ControllerError::MissingBoundary(BoundaryKind::AgentAdapter)
        );
    }

    #[test]
    fn command_envelope_gets_stable_idempotency_key() {
        let command = start_session_command();

        assert_eq!(command.idempotency_key, "Cli:cmd-1");
    }

    fn start_session_command() -> CommandEnvelope {
        CommandEnvelope::new(
            CommandId::new("cmd-1"),
            InputOrigin::Cli,
            "local-user",
            ProjectId::new("project-1"),
            CommandTarget::Agent(AgentId::new("agent-1")),
            CommandIntent::StartSession,
        )
        .with_text("inspect status")
    }
}
