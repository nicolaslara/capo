//! Agent adapter scaffolding.
//!
//! P1 will introduce static dispatch over fake, Codex, Claude Code, and ACP
//! adapter variants.

/// Initial adapter variants named by the architecture.
pub const PLANNED_ADAPTERS: &[&str] = &["fake", "codex-exec", "claude-code", "acp"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planned_adapters_include_fake_and_first_real_targets() {
        assert!(PLANNED_ADAPTERS.contains(&"fake"));
        assert!(PLANNED_ADAPTERS.contains(&"codex-exec"));
        assert!(PLANNED_ADAPTERS.contains(&"claude-code"));
    }
}
