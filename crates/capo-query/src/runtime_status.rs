use capo_state::{ConnectivityExposureProjection, RuntimeTargetProjection};

use crate::{ProjectDashboard, RuntimeTargetControlReadiness};

impl ProjectDashboard {
    pub fn runtime_target_status(
        &self,
        runtime_target_id: &str,
    ) -> Option<&RuntimeTargetProjection> {
        self.runtime_targets
            .iter()
            .rev()
            .find(|target| target.runtime_target_id == runtime_target_id)
    }

    pub fn latest_runtime_target(
        &self,
        runner_kind: Option<&str>,
        status: Option<&str>,
    ) -> Option<&RuntimeTargetProjection> {
        self.runtime_targets
            .iter()
            .filter(|target| {
                runner_kind
                    .map(|kind| runtime_runner_kind_matches(&target.runner_kind, kind))
                    .unwrap_or(true)
            })
            .filter(|target| status.map(|value| target.status == value).unwrap_or(true))
            .max_by(|left, right| {
                left.updated_sequence
                    .cmp(&right.updated_sequence)
                    .then_with(|| left.runtime_target_id.cmp(&right.runtime_target_id))
            })
    }

    pub fn runtime_target_control_readiness(
        &self,
        runtime_target_id: &str,
    ) -> Option<RuntimeTargetControlReadiness> {
        let target = self.runtime_target_status(runtime_target_id)?;
        let latest_control_exposure = self.latest_connectivity_exposure(
            Some("runtime_target"),
            Some(runtime_target_id),
            Some("control"),
        );
        let exposure_ready = latest_control_exposure
            .map(|exposure| exposure.status == "active" && exposure.reachable)
            .unwrap_or(false);
        let target_ready = target.status == "available";
        let ready = target_ready && exposure_ready;
        let mut blockers = Vec::new();
        if !target_ready {
            blockers.push(format!("runtime_target_status_{}", target.status));
        }
        match latest_control_exposure {
            Some(exposure) if exposure.status != "active" => {
                blockers.push(format!("control_exposure_status_{}", exposure.status));
            }
            Some(exposure) if !exposure.reachable => {
                blockers.push("control_exposure_unreachable".to_string());
            }
            Some(_) => {}
            None => blockers.push("control_exposure_missing".to_string()),
        }
        let next_action = if ready {
            "use_runtime_target_for_remote_control"
        } else if !target_ready {
            "set_runtime_target_available"
        } else if latest_control_exposure.is_none() {
            "record_control_connectivity_exposure"
        } else if latest_control_exposure
            .map(|exposure| exposure.status == "blocked_pending_permission")
            .unwrap_or(false)
        {
            "request_or_grant_control_exposure_permission"
        } else {
            "repair_or_replace_control_exposure"
        };

        Some(RuntimeTargetControlReadiness {
            runtime_target_id: target.runtime_target_id.clone(),
            runner_kind: target.runner_kind.clone(),
            target_status: target.status.clone(),
            target_ready,
            control_exposure_ready: exposure_ready,
            control_exposure_id: latest_control_exposure
                .map(|exposure| exposure.exposure_id.clone())
                .unwrap_or_else(|| "none".to_string()),
            control_exposure_status: latest_control_exposure
                .map(|exposure| exposure.status.clone())
                .unwrap_or_else(|| "missing".to_string()),
            control_exposure_scope: latest_control_exposure
                .map(|exposure| exposure.exposure.clone())
                .unwrap_or_else(|| "none".to_string()),
            control_exposure_permission_scope: latest_control_exposure
                .map(|exposure| exposure.permission_scope.clone())
                .unwrap_or_else(|| "none".to_string()),
            control_exposure_reachable: latest_control_exposure
                .map(|exposure| exposure.reachable)
                .unwrap_or(false),
            ready,
            blockers: if blockers.is_empty() {
                "none".to_string()
            } else {
                blockers.join(",")
            },
            next_action: next_action.to_string(),
        })
    }

    pub fn connectivity_exposure_status(
        &self,
        exposure_id: &str,
    ) -> Option<&ConnectivityExposureProjection> {
        self.connectivity_exposures
            .iter()
            .rev()
            .find(|exposure| exposure.exposure_id == exposure_id)
    }

    pub fn latest_connectivity_exposure(
        &self,
        owner_kind: Option<&str>,
        owner_id: Option<&str>,
        channel_kind: Option<&str>,
    ) -> Option<&ConnectivityExposureProjection> {
        self.connectivity_exposures
            .iter()
            .filter(|exposure| {
                owner_kind
                    .map(|kind| exposure.owner_kind == kind)
                    .unwrap_or(true)
            })
            .filter(|exposure| owner_id.map(|id| exposure.owner_id == id).unwrap_or(true))
            .filter(|exposure| {
                channel_kind
                    .map(|channel| exposure.channel_kind == channel)
                    .unwrap_or(true)
            })
            .max_by(|left, right| {
                left.updated_sequence
                    .cmp(&right.updated_sequence)
                    .then_with(|| left.exposure_id.cmp(&right.exposure_id))
            })
    }
}

fn runtime_runner_kind_matches(stored: &str, requested: &str) -> bool {
    stored == requested
        || stored.replace('_', "-") == requested
        || stored.replace('-', "_") == requested
}
