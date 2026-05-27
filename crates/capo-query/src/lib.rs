//! Reusable read-model queries for Capo operator surfaces.
//!
//! This crate owns aggregation over state projections. CLI, dashboards, voice,
//! mobile, and web surfaces should render these structs instead of stitching
//! SQLite read models together independently.

mod adapter_status;
mod dashboard;
mod dogfood;
mod runtime_status;
mod summary;
mod types;

pub use dashboard::project_dashboard;
pub use dogfood::{adapter_dogfood_gate, project_dogfood_readiness};
pub use types::{
    AdapterDispatchStatus, AdapterDogfoodGate, AgentDashboardRow, ProjectDashboard,
    ProjectDashboardQuery, ProjectDogfoodReadiness, RuntimeTargetControlReadiness,
    SessionDashboardRow, SourceTaskProjection, ToolActivitySummary,
};

#[cfg(test)]
mod tests;
