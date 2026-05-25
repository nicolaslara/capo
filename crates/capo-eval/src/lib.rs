//! Evaluation and evidence scaffolding.
//!
//! Later prototype tasks will turn run outcomes and review evidence into
//! inspectable evaluation records.

use capo_core::{BoundaryBinding, BoundaryKind};

/// The first evaluation path is local and evidence-backed.
pub const PROTOTYPE_EVALUATION_LAYER: &str = "local-evidence-report";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvaluationLayer {
    Fake(FakeEvaluationLayer),
}

impl EvaluationLayer {
    pub fn fake() -> Self {
        Self::Fake(FakeEvaluationLayer)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(layer) => layer.binding(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeEvaluationLayer;

impl FakeEvaluationLayer {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::EvaluationLayer, "fake-eval")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prototype_evaluation_layer_is_local() {
        assert_eq!(PROTOTYPE_EVALUATION_LAYER, "local-evidence-report");
    }

    #[test]
    fn fake_evaluation_reports_eval_boundary() {
        assert_eq!(
            EvaluationLayer::fake().binding().kind,
            BoundaryKind::EvaluationLayer
        );
    }
}
