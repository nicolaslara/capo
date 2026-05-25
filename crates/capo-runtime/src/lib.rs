//! Runtime runner scaffolding.
//!
//! P5 will add fake and local process runners while keeping runtime execution
//! separate from connectivity/tunnel concerns.

/// First runtime variants from the prototype plan.
pub const PLANNED_RUNTIMES: &[&str] = &["fake", "local-process"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planned_runtimes_keep_fake_and_local_process() {
        assert_eq!(PLANNED_RUNTIMES, ["fake", "local-process"]);
    }
}
