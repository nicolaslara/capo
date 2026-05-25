use std::env;

const HELP: &str = "\
Capo - local controller for coding-agent sessions

Usage:
  capo --help
  capo version

Prototype commands planned:
  init             Initialize local Capo state
  agent register   Register a fake or local agent
  task send        Send a goal to an agent session
  session status   Inspect session state from read models
  session redirect Steer an active session toward a new goal
  session interrupt Interrupt an active session
  recover          Rebuild projections and reconcile runtime state
  evidence export  Export human-auditable run evidence

This P0 scaffold does not read provider credentials, start agents, or create state.
";

fn main() {
    let mut args = env::args().skip(1);

    match args.next().as_deref() {
        None | Some("--help" | "-h" | "help") => print!("{HELP}"),
        Some("version") | Some("--version") | Some("-V") => {
            println!("capo {}", env!("CARGO_PKG_VERSION"));
        }
        Some(command) => {
            eprintln!("unknown command: {command}");
            eprintln!("run `capo --help` for the P0 command skeleton");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HELP;
    use capo_adapters::{AgentAdapter, ProviderConnector};
    use capo_core::{
        AgentId, BoundarySet, CapoController, CommandEnvelope, CommandId, CommandIntent,
        CommandTarget, InputOrigin, ProjectId, SessionStatus,
    };
    use capo_eval::EvaluationLayer;
    use capo_memory::MemoryBackend;
    use capo_runtime::{ConnectivityTunnel, RuntimeRunner};
    use capo_state::StateStore;
    use capo_tools::{PermissionPolicy, ToolExposure};

    #[test]
    fn help_mentions_no_credentials() {
        assert!(HELP.contains("does not read provider credentials"));
    }

    #[test]
    fn help_lists_recovery_and_evidence_commands() {
        assert!(HELP.contains("recover"));
        assert!(HELP.contains("evidence export"));
    }

    #[test]
    fn fake_boundaries_wire_through_controller_without_persistence() {
        let mut boundaries = BoundarySet::new();
        boundaries.register(AgentAdapter::fake().binding());
        boundaries.register(RuntimeRunner::fake().binding());
        boundaries.register(ConnectivityTunnel::fake().binding());
        boundaries.register(ProviderConnector::fake().binding());
        boundaries.register(PermissionPolicy::fake().binding());
        boundaries.register(ToolExposure::fake().binding());
        boundaries.register(StateStore::fake().binding());
        boundaries.register(MemoryBackend::fake().binding());
        boundaries.register(EvaluationLayer::fake().binding());

        let command = CommandEnvelope::new(
            CommandId::new("cmd-wire-fake"),
            InputOrigin::Cli,
            "local-user",
            ProjectId::new("project-capo"),
            CommandTarget::Agent(AgentId::new("agent-fake")),
            CommandIntent::StartSession,
        )
        .with_text("inspect current workpad status");

        let preview = CapoController::new()
            .preview_start_session(&command, &boundaries)
            .expect("all fake boundaries should satisfy controller preview");

        assert!(boundaries.all_fake());
        assert_eq!(preview.boundary_count, 9);
        assert_eq!(preview.session.status, SessionStatus::Starting);
        assert_eq!(
            preview.session.current_goal,
            "inspect current workpad status"
        );
        assert_eq!(preview.run.session_id, preview.session.session_id);
        assert_eq!(preview.turn.session_id, preview.session.session_id);
    }
}
