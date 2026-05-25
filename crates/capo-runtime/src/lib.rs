//! Runtime runner and connectivity scaffolding.
//!
//! The fake variants let controller tests prove runtime/tunnel separation before
//! local process execution or remote connectivity exists.

use capo_core::{BoundaryBinding, BoundaryKind};

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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeRuntimeRunner;

impl FakeRuntimeRunner {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::RuntimeRunner, "fake-runtime")
    }
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
