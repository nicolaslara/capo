//! Evaluation and evidence scaffolding.
//!
//! Later prototype tasks will turn run outcomes and review evidence into
//! inspectable evaluation records.

/// The first evaluation path is local and evidence-backed.
pub const PROTOTYPE_EVALUATION_LAYER: &str = "local-evidence-report";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prototype_evaluation_layer_is_local() {
        assert_eq!(PROTOTYPE_EVALUATION_LAYER, "local-evidence-report");
    }
}
