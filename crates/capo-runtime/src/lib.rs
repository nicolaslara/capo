//! Runtime runner and connectivity scaffolding.
//!
//! The fake variants let controller tests prove runtime/tunnel separation before
//! local process execution or remote connectivity exists.

use capo_core::{BoundaryBinding, BoundaryKind, RunId};

/// First runtime variants from the prototype plan.
pub const PLANNED_RUNTIMES: &[&str] = &["fake", "local-process"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeRunner {
    Fake(FakeRuntimeRunner),
}

impl RuntimeRunner {
    pub fn fake() -> Self {
        Self::Fake(FakeRuntimeRunner)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(runner) => runner.binding(),
        }
    }

    pub fn start(&self, request: FakeRuntimeStartRequest) -> FakeRuntimeProcess {
        match self {
            Self::Fake(runner) => runner.start(request),
        }
    }

    pub fn interrupt(&self, process: &FakeRuntimeProcess, reason: &str) -> FakeRuntimeProcess {
        match self {
            Self::Fake(runner) => runner.interrupt(process, reason),
        }
    }

    pub fn attach_process(&self, run_id: RunId, runtime_process_ref: String) -> FakeRuntimeProcess {
        match self {
            Self::Fake(runner) => runner.attach_process(run_id, runtime_process_ref),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeRuntimeRunner;

impl FakeRuntimeRunner {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::RuntimeRunner, "fake-runtime")
    }

    pub fn start(&self, request: FakeRuntimeStartRequest) -> FakeRuntimeProcess {
        FakeRuntimeProcess {
            run_id: request.run_id,
            runtime_process_ref: format!("fake-runtime-process-{}", request.agent_name),
            status: "running".to_string(),
        }
    }

    pub fn interrupt(&self, process: &FakeRuntimeProcess, _reason: &str) -> FakeRuntimeProcess {
        FakeRuntimeProcess {
            run_id: process.run_id.clone(),
            runtime_process_ref: process.runtime_process_ref.clone(),
            status: "stopping".to_string(),
        }
    }

    pub fn attach_process(&self, run_id: RunId, runtime_process_ref: String) -> FakeRuntimeProcess {
        FakeRuntimeProcess {
            run_id,
            runtime_process_ref,
            status: "running".to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeRuntimeStartRequest {
    pub run_id: RunId,
    pub agent_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeRuntimeProcess {
    pub run_id: RunId,
    pub runtime_process_ref: String,
    pub status: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectivityTunnel {
    Fake(FakeTunnel),
}

impl ConnectivityTunnel {
    pub fn fake() -> Self {
        Self::Fake(FakeTunnel)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(tunnel) => tunnel.binding(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeTunnel;

impl FakeTunnel {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::ConnectivityTunnel, "fake-tunnel")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planned_runtimes_keep_fake_and_local_process() {
        assert_eq!(PLANNED_RUNTIMES, ["fake", "local-process"]);
    }

    #[test]
    fn fake_runtime_and_tunnel_are_separate_boundaries() {
        assert_eq!(
            RuntimeRunner::fake().binding().kind,
            BoundaryKind::RuntimeRunner
        );
        assert_eq!(
            ConnectivityTunnel::fake().binding().kind,
            BoundaryKind::ConnectivityTunnel
        );
    }
}
