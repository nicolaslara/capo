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

    #[test]
    fn help_mentions_no_credentials() {
        assert!(HELP.contains("does not read provider credentials"));
    }

    #[test]
    fn help_lists_recovery_and_evidence_commands() {
        assert!(HELP.contains("recover"));
        assert!(HELP.contains("evidence export"));
    }
}
