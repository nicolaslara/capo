//! Tool exposure and instrumentation scaffolding.
//!
//! P8 will add Capo-owned tools and durable instrumentation records.

/// First Capo-owned tools selected by the architecture.
pub const CAPO_OWNED_TOOLS: &[&str] = &[
    "capo.task_status",
    "capo.agent_status",
    "capo.session_summary",
    "capo.workpad_read",
    "capo.evidence_record",
    "capo.capability_request",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_tool_set_supports_status_and_evidence() {
        assert!(CAPO_OWNED_TOOLS.contains(&"capo.task_status"));
        assert!(CAPO_OWNED_TOOLS.contains(&"capo.evidence_record"));
    }
}
