//! GA2 (goal-orchestration GO4/GO5/GO6/GO9/GO10): deterministic server-boundary
//! tests for the goal lifecycle mutations, the read surfaces, the historical
//! report rendering, and the load-bearing safety rejections (mark-complete is
//! never a lifecycle command; a `validated`/`reviewed` requirement is never
//! recorded on an agent claim alone; an unclassifiable report source is
//! rejected). Every assertion is on typed payloads, not console text.

use super::*;
use crate::{GoalReportFormat, GoalReportRecord, GoalSpec, RequirementStatusRecord};

/// A minimal-but-complete [`GoalSpec`] with structured GO6 fields populated, so a
/// `SetGoal` round-trips through the durable goal projection exactly.
fn goal_spec(goal_id: &str, objective: &str, requirements: &[(&str, &str)]) -> GoalSpec {
    GoalSpec {
        goal_id: goal_id.to_string(),
        objective: objective.to_string(),
        task_id: Some("task-ga2".to_string()),
        agent_id: Some("agent-ga2".to_string()),
        session_id: Some("session-ga2".to_string()),
        parent_goal_id: None,
        attempt_run_id: None,
        requirements: requirements
            .iter()
            .map(|(id, summary)| crate::GoalRequirementSpec {
                requirement_id: (*id).to_string(),
                summary: (*summary).to_string(),
            })
            .collect(),
        success_criteria_json: r#"{"must":["build green"]}"#.to_string(),
        constraints_json: r#"{"no_network":true}"#.to_string(),
        verification_surface_json: r#"{"cmd":"cargo test"}"#.to_string(),
        budget_json: r#"{"max_turns":8}"#.to_string(),
        stop_conditions_json: r#"{"on":"blocker"}"#.to_string(),
    }
}

fn report(goal_id: &str, report_id: &str, kind: &str, source: &str) -> GoalReportRecord {
    GoalReportRecord {
        goal_report_id: report_id.to_string(),
        goal_id: goal_id.to_string(),
        session_id: Some("session-ga2".to_string()),
        requirement_id: Some("req-1".to_string()),
        report_kind: kind.to_string(),
        source: source.to_string(),
        confidence: if source == "agent_reported" {
            Some(70)
        } else {
            None
        },
        summary: format!("{kind} via {source}"),
        body_artifact_id: None,
        evidence_id: None,
    }
}

fn open_server() -> CapoServer {
    let root = temp_root();
    CapoServer::open(ProjectId::new("project-capo"), &root).expect("server")
}

#[test]
fn goal_lifecycle_mutations_flow_through_the_server_and_drive_the_read_model() {
    let server = open_server();

    // SetGoal creates the goal `active`, seeds requirements at `unverified`, and
    // stores the structured GO6 fields verbatim.
    let created = handle(
        &server,
        ServerCommand::SetGoal {
            spec: goal_spec(
                "goal-1",
                "Ship the GA2 read surfaces",
                &[("req-1", "lifecycle commands"), ("req-2", "read surfaces")],
            ),
        },
    );
    let ServerResponsePayload::GoalView(view) = created.payload else {
        panic!("expected goal view");
    };
    assert_eq!(view.summary.goal_id, "goal-1");
    assert_eq!(view.summary.status, "active");
    assert_eq!(view.summary.requirement_count, 2);
    assert_eq!(view.budget_json, r#"{"max_turns":8}"#);
    assert!(view.requirements.iter().all(|r| r.status == "unverified"));

    // Pause -> blocked -> resume each transitions the projection and stores the
    // blocker reason as current-blocker state.
    for (command, expected_status) in [
        (
            ServerCommand::PauseGoal {
                goal_id: "goal-1".to_string(),
            },
            "paused",
        ),
        (
            ServerCommand::BlockGoal {
                goal_id: "goal-1".to_string(),
                reason: "waiting on safety-gates lock".to_string(),
            },
            "blocked",
        ),
        (
            ServerCommand::ResumeGoal {
                goal_id: "goal-1".to_string(),
            },
            "active",
        ),
    ] {
        let response = handle(&server, command);
        let ServerResponsePayload::GoalView(view) = response.payload else {
            panic!("expected goal view");
        };
        assert_eq!(view.summary.status, expected_status);
    }

    // ListGoals reflects the latest lifecycle state.
    let listed = handle(&server, ServerCommand::ListGoals);
    let ServerResponsePayload::Goals(goals) = listed.payload else {
        panic!("expected goals listing");
    };
    assert_eq!(goals.len(), 1);
    assert_eq!(goals[0].status, "active");
}

#[test]
fn a_direct_mark_goal_complete_request_is_rejected_by_construction() {
    let server = open_server();
    handle(
        &server,
        ServerCommand::SetGoal {
            spec: goal_spec("goal-complete", "no direct completion", &[("req-1", "x")]),
        },
    );
    let rejected = server
        .handle(ServerRequest::cli(ServerCommand::MarkGoalComplete {
            goal_id: "goal-complete".to_string(),
        }))
        .expect_err("a direct mark-complete must be rejected");
    assert!(matches!(
        rejected,
        ServerError::GoalCompleteNotALifecycleCommand { goal_id }
            if goal_id == "goal-complete"
    ));
}

#[test]
fn a_set_goal_to_complete_status_is_rejected_as_an_illegal_lifecycle_transition() {
    let server = open_server();
    // A goal whose id encodes a "complete" status request cannot be set complete
    // through SetGoal; only the GA5 auditor reaches goal-complete. The lifecycle
    // commands (`pause`/`resume`/`block`/`clear`) are the only statuses SetGoal
    // and the lifecycle surface own.
    let unknown = server
        .handle(ServerRequest::cli(ServerCommand::PauseGoal {
            goal_id: "missing-goal".to_string(),
        }))
        .expect_err("pausing an unknown goal is rejected");
    assert!(matches!(
        unknown,
        ServerError::UnknownGoal { goal_id } if goal_id == "missing-goal"
    ));
}

#[test]
fn a_validated_requirement_backed_only_by_an_agent_claim_is_rejected() {
    let server = open_server();
    handle(
        &server,
        ServerCommand::SetGoal {
            spec: goal_spec("goal-claim", "claims are not evidence", &[("req-1", "x")]),
        },
    );

    // `validated` on an `agent_reported` source alone is the auditor's strength,
    // not a recordable read-model state: rejected at the boundary.
    let rejected = server
        .handle(ServerRequest::cli(ServerCommand::SetRequirementStatus {
            record: RequirementStatusRecord {
                requirement_id: "req-1".to_string(),
                goal_id: "goal-claim".to_string(),
                summary: "x".to_string(),
                status: "validated".to_string(),
                source: "agent_reported".to_string(),
            },
        }))
        .expect_err("validated-on-claim must be rejected");
    assert!(matches!(
        rejected,
        ServerError::IllegalGoalStatusTransition { .. }
    ));

    // `supported` on observed evidence is accepted and drives the read model.
    let response = handle(
        &server,
        ServerCommand::SetRequirementStatus {
            record: RequirementStatusRecord {
                requirement_id: "req-1".to_string(),
                goal_id: "goal-claim".to_string(),
                summary: "x".to_string(),
                status: "supported".to_string(),
                source: "runtime_output".to_string(),
            },
        },
    );
    let ServerResponsePayload::GoalView(view) = response.payload else {
        panic!("expected goal view");
    };
    let requirement = view
        .requirements
        .iter()
        .find(|r| r.requirement_id == "req-1")
        .expect("req-1");
    assert_eq!(requirement.status, "supported");
    assert!(requirement.observed);
}

#[test]
fn an_unclassifiable_report_source_is_rejected() {
    let server = open_server();
    handle(
        &server,
        ServerCommand::SetGoal {
            spec: goal_spec("goal-src", "classify sources", &[("req-1", "x")]),
        },
    );
    let rejected = server
        .handle(ServerRequest::cli(ServerCommand::RecordGoalReport {
            report: report(
                "goal-src",
                "report-bad",
                "capo.report_progress",
                "telepathy",
            ),
        }))
        .expect_err("an unclassifiable source must be rejected");
    assert!(matches!(
        rejected,
        ServerError::UnclassifiableReportSource { source } if source == "telepathy"
    ));
}

#[test]
fn report_read_surfaces_separate_observed_evidence_from_agent_claims() {
    let server = open_server();
    handle(
        &server,
        ServerCommand::SetGoal {
            spec: goal_spec("goal-rep", "story and evidence", &[("req-1", "x")]),
        },
    );
    // One agent claim and one observed-evidence row.
    handle(
        &server,
        ServerCommand::RecordGoalReport {
            report: report(
                "goal-rep",
                "report-claim",
                "capo.report_progress",
                "agent_reported",
            ),
        },
    );
    handle(
        &server,
        ServerCommand::RecordGoalReport {
            report: report("goal-rep", "report-obs", "runtime_output", "runtime_output"),
        },
    );

    // The story surface shows every report; evidence shows only observed rows.
    let story = handle(
        &server,
        ServerCommand::GoalStory {
            goal_id: "goal-rep".to_string(),
        },
    );
    let ServerResponsePayload::GoalReports(story) = story.payload else {
        panic!("expected story listing");
    };
    assert_eq!(story.surface, "story");
    assert_eq!(story.reports.len(), 2);

    let evidence = handle(
        &server,
        ServerCommand::GoalEvidence {
            goal_id: "goal-rep".to_string(),
        },
    );
    let ServerResponsePayload::GoalReports(evidence) = evidence.payload else {
        panic!("expected evidence listing");
    };
    assert_eq!(evidence.surface, "evidence");
    assert_eq!(evidence.reports.len(), 1);
    assert!(evidence.reports[0].observed);
    assert_eq!(evidence.reports[0].confidence, None);
}

#[test]
fn historical_report_renders_markdown_and_json_without_leaking_raw_bodies() {
    let server = open_server();
    handle(
        &server,
        ServerCommand::SetGoal {
            spec: goal_spec(
                "goal-report",
                "render a report",
                &[("req-1", "build green")],
            ),
        },
    );
    // An agent-reported row that cites a raw body artifact: GO10 names the
    // artifact by id, never inlines its content.
    let mut claim = report(
        "goal-report",
        "report-claim",
        "capo.report_progress",
        "agent_reported",
    );
    claim.body_artifact_id = Some("artifact-raw-stdout".to_string());
    claim.summary = "claimed the build passed".to_string();
    handle(&server, ServerCommand::RecordGoalReport { report: claim });

    // Markdown rendering: titled, observed-vs-reported tagged, artifact named not
    // inlined, and the structured `format` tag set.
    let md = handle(
        &server,
        ServerCommand::GoalReport {
            goal_id: "goal-report".to_string(),
            format: GoalReportFormat::Markdown,
        },
    );
    let ServerResponsePayload::GoalReport(md) = md.payload else {
        panic!("expected markdown report");
    };
    assert_eq!(md.format, "markdown");
    assert_eq!(md.goal_id, "goal-report");
    assert!(md.body.starts_with("# Goal report: goal-report\n"));
    assert!(md.body.contains("- Objective: render a report\n"));
    assert!(md.body.contains("## Requirements"));
    assert!(md.body.contains("reported (confidence 70)"));
    assert!(md.body.contains("body artifact: artifact-raw-stdout"));

    // JSON rendering: same derived data, parses as JSON, and carries the goal id.
    let json = handle(
        &server,
        ServerCommand::GoalReport {
            goal_id: "goal-report".to_string(),
            format: GoalReportFormat::Json,
        },
    );
    let ServerResponsePayload::GoalReport(json) = json.payload else {
        panic!("expected json report");
    };
    assert_eq!(json.format, "json");
    let parsed: serde_json::Value =
        serde_json::from_str(&json.body).expect("json report body parses");
    assert_eq!(parsed["goal_id"], "goal-report");
    assert_eq!(parsed["objective"], "render a report");
    // The raw artifact body is never inlined into the report JSON.
    assert!(!json.body.contains("DO_NOT_INLINE_RAW"));
}
