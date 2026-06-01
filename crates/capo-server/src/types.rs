use capo_controller::FakeRunRefs;
use capo_core::{AgentId, InputOrigin, ProjectId, RunId, SessionId, TaskId};
use capo_state::{
    AdapterDispatchExecutionProjection, AdapterDispatchPlanProjection,
    AdapterDispatchPromptSourceProjection,
};

use crate::util::{slug, stable_hash};

pub type ServerResult<T> = Result<T, ServerError>;

#[derive(Debug)]
pub enum ServerError {
    State(capo_state::StateError),
    AdapterFixture(String),
    UnknownAgent {
        agent_name: String,
    },
    AgentHasNoActiveSession {
        agent_name: String,
    },
    AgentAlreadyHasSession {
        agent_name: String,
        session_id: String,
        run_status: Option<String>,
    },
    SessionAlreadyExists {
        session_id: String,
    },
    RunAlreadyExists {
        run_id: String,
    },
    UnknownDispatchPlan {
        dispatch_plan_id: String,
    },
    UnknownSession {
        session_id: String,
    },
    RunSessionMismatch {
        session_id: String,
        run_id: String,
        actual_session_id: String,
    },
    AdapterSessionMismatch {
        session_id: String,
        session_adapter: String,
        requested_adapter: String,
    },
    /// AI2: `RegisterAgent` requested a chat adapter binding other than the two
    /// supported values (`fake`, `codex`). Rejected before the agent is created.
    UnsupportedChatAdapter {
        adapter: String,
    },
    /// GA2: a goal lifecycle mutation referenced a goal that does not exist.
    UnknownGoal {
        goal_id: String,
    },
    /// GA2 (goal-orchestration GO9): a request tried to drive a goal to a Capo
    /// goal-complete transition through an ordinary lifecycle command. Completion
    /// is reachable ONLY through the GA5 evidence-gated auditor; an agent claim or
    /// a direct "mark complete" is recorded as evidence and never flips goal
    /// state. Rejected by construction so completion is never reachable by
    /// assertion alone.
    GoalCompleteNotALifecycleCommand {
        goal_id: String,
    },
    /// GA2: a goal lifecycle mutation requested a status the lifecycle surface
    /// does not own (e.g. `complete`, or an unrecognized status). The lifecycle
    /// statuses are `active` / `paused` / `blocked` / `cleared` only.
    IllegalGoalStatusTransition {
        goal_id: String,
        requested_status: String,
    },
    /// GA2: a goal report/evidence/review/validation record carried a `source`
    /// tag that is neither an agent claim nor a recognized observed-evidence
    /// source, so it could not be classified observed-vs-reported.
    UnclassifiableReportSource {
        source: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerRequest {
    pub request_id: String,
    pub origin: ServerClientOrigin,
    pub command: ServerCommand,
}

impl ServerRequest {
    pub fn cli(command: ServerCommand) -> Self {
        Self::local_cli(default_request_id(&command), command)
    }

    pub fn local_cli(request_id: impl Into<String>, command: ServerCommand) -> Self {
        Self {
            request_id: request_id.into(),
            origin: ServerClientOrigin {
                client_id: "local-cli".to_string(),
                actor_id: "local-user".to_string(),
                input_origin: ServerInputOrigin::Cli,
            },
            command,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerClientOrigin {
    pub client_id: String,
    pub actor_id: String,
    pub input_origin: ServerInputOrigin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerInputOrigin {
    Cli,
    Dashboard,
    Mobile,
    Voice,
    Api,
    System,
}

impl From<ServerInputOrigin> for InputOrigin {
    fn from(value: ServerInputOrigin) -> Self {
        match value {
            ServerInputOrigin::Cli => Self::Cli,
            ServerInputOrigin::Dashboard => Self::Dashboard,
            ServerInputOrigin::Mobile => Self::Mobile,
            ServerInputOrigin::Voice => Self::Voice,
            ServerInputOrigin::Api => Self::Api,
            ServerInputOrigin::System => Self::System,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServerCommand {
    RegisterAgent {
        name: String,
        /// AI2: the agent's chat adapter binding. `fake` (the default) keeps the
        /// deterministic fake adapter for `SendTask`/`SteerAgent`; `codex` binds
        /// the real read-only one-shot [`capo_adapters::CodexLiveAdapter`] for
        /// THIS agent's chat turns (fail-closed-fast when the live-provider gate
        /// is off). Binding is per-agent: Codex is never a global chat default.
        adapter: String,
    },
    SendTask {
        agent_name: String,
        goal: String,
        scenario: String,
    },
    SteerAgent {
        agent_name: String,
        goal: String,
    },
    InterruptAgent {
        agent_name: String,
        reason: String,
    },
    StopAgent {
        agent_name: String,
        reason: String,
    },
    ListAgents,
    AgentStatus {
        agent_name: String,
    },
    Dashboard {
        recent_event_limit: usize,
    },
    StartSession {
        agent_name: String,
        goal: String,
        adapter: String,
        session_id: Option<String>,
        run_id: Option<String>,
    },
    ReplayAdapterFixture {
        adapter: String,
        session_id: String,
        run_id: String,
        turn_id: String,
        fixture_name: String,
        fixture_jsonl: String,
    },
    PlanDispatch {
        agent_name: String,
        adapter: String,
        goal: String,
        workspace: String,
        artifacts: String,
        session_id: String,
        run_id: String,
        turn_id: String,
        deterministic_opt_in: bool,
    },
    PreflightLiveProvider {
        agent_name: String,
        adapter: String,
        goal: String,
        workspace: String,
        artifacts: String,
        session_id: String,
        run_id: String,
        turn_id: String,
        capability_profile: String,
        runtime_scope: String,
        credential_scan_policy: String,
        raw_prompt_policy: String,
        raw_output_policy: String,
        tool_wrapper_policy: String,
        live_provider_opt_in: bool,
    },
    GateDispatch {
        dispatch_plan_id: String,
    },
    RunDispatchLocal {
        dispatch_plan_id: String,
        fixture_name: String,
        fixture_jsonl: String,
    },
    RunLiveProviderLocal {
        dispatch_plan_id: String,
        goal: String,
        live_execution_opt_in: bool,
        mock_runtime_opt_in: bool,
        mock_provider_output_name: Option<String>,
        mock_provider_output_jsonl: Option<String>,
        timeout_seconds: u64,
        /// Absolute path to a codex binary to run on the spawn path instead of
        /// resolving `codex` from PATH. Ops set it from `CAPO_CODEX_BIN`; tests
        /// pass a stub so the spawn path is deterministic. `None`/relative keeps
        /// `codex`.
        codex_program_override: Option<String>,
        /// RTL9: whether this turn is running unattended. An unattended turn can
        /// never reach a live workspace write here (that is `goal-autonomy`
        /// territory); the handler resolves the write mode via the RTL6 gate
        /// (`live_execution_opt_in` AND `CAPO_SERVER_RUN_CODEX_LIVE` AND
        /// `!unattended`), so a `true` here forces the read-only dry-run profile.
        unattended: bool,
    },
    /// AI1: drive ONE turn through the single production orchestration path
    /// ([`crate::CapoServer::run_dispatch_turn`]) -- the loop DRIVES
    /// preflight/gate/run over the dispatch primitives and ANNOTATES the run it
    /// drove with a `TurnFinished`. This is the production command the
    /// operator/live-run flow issues so the loop the design is built around is the
    /// path that executes, instead of hand-sequencing
    /// `PreflightLiveProvider` + `RunLiveProviderLocal` beside the loop.
    ///
    /// It carries the live-provider turn inputs plus the RTL7 resource ceiling and
    /// per-run usage as flat scalars (the handler rebuilds
    /// [`capo_controller::RunResourceCeiling`]/[`capo_controller::RunResourceUsage`]),
    /// so the wire stays simple and the loop's ceiling enforcement is honored on
    /// the production path.
    RunDispatchTurn {
        agent_name: String,
        adapter: String,
        goal: String,
        workspace: String,
        artifacts: String,
        session_id: String,
        run_id: String,
        turn_id: String,
        capability_profile: String,
        runtime_scope: String,
        credential_scan_policy: String,
        raw_prompt_policy: String,
        raw_output_policy: String,
        tool_wrapper_policy: String,
        live_provider_opt_in: bool,
        live_execution_opt_in: bool,
        mock_runtime_opt_in: bool,
        mock_provider_output_name: Option<String>,
        mock_provider_output_jsonl: Option<String>,
        /// Wall-clock bound (seconds) for the live turn. A live turn MUST run
        /// inside a wall-clock-bounded ceiling, so this is required; the handler
        /// rejects a zero. Wired to the runtime timeout.
        timeout_seconds: u64,
        /// Turns ceiling for the run (the loop counts the turn about to run).
        max_turns: u32,
        /// Token/cost ceiling for the run, in provider cost units.
        max_token_cost: u64,
        /// Per-run turns already taken before this turn (carried across turns).
        turns_taken_before: u32,
        /// Per-run token/cost already accrued before this turn.
        token_cost_before: u64,
        /// Pre-turn token/cost estimate for the turn about to run.
        turn_token_cost: u64,
        /// RTL9: whether this turn runs unattended (forces the read-only dry-run
        /// profile; a live workspace write needs an attended run).
        unattended: bool,
    },
    Recover,
    /// Tail the append-only event log (ST4). The subscriber catches up on the
    /// backlog strictly after `from_sequence` (optionally filtered to one
    /// session) and then receives newly-committed events live. A `None`
    /// `session_id` tails every committed event; `Some(id)` tails one session.
    Subscribe {
        session_id: Option<String>,
        from_sequence: i64,
    },
    /// Read a session's multi-turn conversation thread (ST5), a read model
    /// projected from the event log, incrementally by sequence. The thread
    /// reconstructs strictly after `from_sequence`, so it composes with
    /// [`ServerCommand::Subscribe`] over the same watermark (read the thread
    /// once, then tail the live events to extend it). `from_sequence` of `0`
    /// reads the full thread.
    ReadThread {
        session_id: String,
        from_sequence: i64,
    },
    // GA2 (goal-orchestration GO4/GO6): the typed goal lifecycle mutations. Every
    // goal mutation flows through this server/controller boundary -- the CLI and
    // any other client are read-only over goals and never own goal state. None of
    // these can transition a goal to `complete`: completion is the GA5 auditor's
    // verdict (see [`ServerCommand::MarkGoalComplete`], which is rejected).
    /// Create or set a goal (GO6). Links the goal to its project, task, agent,
    /// session, and parent goal, seeds its requirements, and stores the
    /// structured success criteria, constraints, verification surface, budget,
    /// and stop conditions. Idempotent on `goal_id`: re-issuing updates in place.
    SetGoal {
        spec: GoalSpec,
    },
    /// Pause an active goal (GO4): it stops being eligible for continuation until
    /// resumed. Does not clear requirements or evidence.
    PauseGoal {
        goal_id: String,
    },
    /// Resume a paused or blocked goal (GO4) back to `active`.
    ResumeGoal {
        goal_id: String,
    },
    /// Mark a goal blocked with a reason (GO4). The reason is stored as
    /// current-blocker state on the goal projection.
    BlockGoal {
        goal_id: String,
        reason: String,
    },
    /// Clear / cancel a goal (GO4): a terminal-but-not-complete lifecycle state.
    ClearGoal {
        goal_id: String,
        reason: String,
    },
    /// Record a per-requirement status transition (GO3). GA2 RECORDS the
    /// transition; the GA5 auditor owns deciding which transition observed
    /// evidence warrants. A `validated`/`reviewed` status backed only by an
    /// `agent_reported` source is rejected here (the read model must never show a
    /// requirement validated by a claim alone).
    SetRequirementStatus {
        record: RequirementStatusRecord,
    },
    /// Record a report / evidence / review / validation event against a goal
    /// (GO4), source-tagged `agent_reported` (a claim) vs an observed source
    /// (`runtime_output` / `adapter_event`). This is the spine behind the story,
    /// evidence, review, validation, and risk read surfaces. The raw body is kept
    /// in an artifact, not as authoritative read-model truth.
    RecordGoalReport {
        report: GoalReportRecord,
    },
    /// GA2 (goal-orchestration GO9): the ONLY "complete this goal" request the
    /// server accepts is this one, and it is REJECTED by construction. Goal
    /// completion is reachable only through the GA5 evidence-gated auditor; this
    /// command exists so the rejection is an explicit, tested contract rather than
    /// a silent absence.
    MarkGoalComplete {
        goal_id: String,
    },
    /// List the project's goals with a concise status summary (GO4/GO5).
    ListGoals,
    /// View one goal in full (GO4): lifecycle, requirements, story, continuation
    /// decisions, and observed delegated-provider state.
    ViewGoal {
        goal_id: String,
    },
    /// The agent story / report ledger for a goal (GO3/GO5), oldest first.
    GoalStory {
        goal_id: String,
    },
    /// The event timeline for a goal (GO5/GO10): the goal's own events plus the
    /// events of its attempt run, in sequence order.
    GoalTimeline {
        goal_id: String,
    },
    /// The observed-evidence report rows for a goal (GO5).
    GoalEvidence {
        goal_id: String,
    },
    /// The validation-kind report rows for a goal (GO5).
    GoalValidations {
        goal_id: String,
    },
    /// The review-kind report rows for a goal (GO5).
    GoalReviews {
        goal_id: String,
    },
    /// The risk / blocker / contradiction surface for a goal (GO5): the goal's
    /// current blocker, blocked/contradicted requirements, and raised-blocker
    /// reports.
    GoalRisks {
        goal_id: String,
    },
    /// A historical execution report for a goal (GO10), rebuildable from events,
    /// projections, and artifacts, rendered as `markdown` or `json`. Degrades
    /// clearly when artifacts are missing or redacted.
    GoalReport {
        goal_id: String,
        format: GoalReportFormat,
    },
    /// AI5 (architecture-improvements): close the autonomy loop. Evaluate the GA4
    /// continuation scheduler for an active goal and, ONLY on a `Continue` decision
    /// AND ONLY when continuation is explicitly enabled, drive exactly ONE follow-on
    /// turn through the SINGLE production orchestration path
    /// ([`crate::CapoServer::run_dispatch_turn`], the same path AI1's
    /// [`ServerCommand::RunDispatchTurn`] uses) -- never a parallel turn driver.
    ///
    /// The command evaluates AND durably records the decision through the GA4
    /// [`capo_controller::FakeBoundaryController::evaluate_and_record_continuation`]
    /// (event + `GoalContinuationProjection`), then branches:
    ///
    /// - `Continue` (only reachable with `conditions.enabled = true` and every
    ///   safe-boundary precondition met): issue ONE `run_dispatch_turn` for the
    ///   goal's attempt run, returning the SAME `DispatchTurn` outcome (run +
    ///   `TurnFinished`) an operator turn produces.
    /// - `Pause` / `Block` / `NoProgressSuppress`: record only; NO turn is driven.
    /// - `BudgetLimit`: record AND durably abort the goal's attempt run via the RTL7
    ///   `run.aborted` path; NO turn is driven.
    ///
    /// Opt-in / off by default: with `conditions.enabled = false` the scheduler
    /// short-circuits to `pause` and this command NEVER dispatches a turn, so the
    /// off-by-default autonomy invariant holds at the production boundary.
    ContinueGoal {
        goal_id: String,
        /// A stable id for this continuation evaluation; recording is idempotent on
        /// `(goal, continuation_id)`.
        continuation_id: String,
        /// The live safe-boundary conditions + explicit enablement the GA4
        /// scheduler decides over. `enabled = false` keeps the loop closed.
        conditions: ContinueGoalConditions,
        /// The dispatch-turn inputs for the follow-on turn driven on a `Continue`.
        /// Reused verbatim by the production `run_dispatch_turn` path so a continued
        /// turn is indistinguishable from an operator turn.
        turn: Box<ContinueGoalTurn>,
    },
}

/// AI5: the live safe-boundary conditions + explicit enablement a
/// [`ServerCommand::ContinueGoal`] folds into the GA4
/// [`capo_controller::ContinuationConditions`]. Mirrors that struct as flat scalars
/// so the wire stays simple; `enabled` is the opt-in (off by default) gate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContinueGoalConditions {
    pub enabled: bool,
    pub runtime_idle: bool,
    pub session_idle: bool,
    pub user_input_queued: bool,
    pub permission_pending: bool,
    pub capability_profile_valid: bool,
    pub next_step_writes_source: bool,
    pub checkpoint_boundary_available: bool,
    pub verification_runner_available: bool,
    pub last_continuation_made_no_progress: bool,
    pub strategy_changed_since_suppression: bool,
    /// Goal-level budget ceiling (turns / wall-clock seconds / token-cost) and the
    /// usage accrued across the continuation series. Composes the RTL7 run ceiling.
    pub budget_max_turns: u32,
    pub budget_timeout_seconds: u64,
    pub budget_max_token_cost: u64,
    pub budget_turns_taken: u32,
    pub budget_token_cost: u64,
}

/// AI5: the dispatch-turn inputs for the follow-on turn a `ContinueGoal` drives on
/// a `Continue` decision. These are exactly the [`ServerCommand::RunDispatchTurn`]
/// inputs (minus the `goal_id`/`continuation_id`, which the command carries) so the
/// continued turn re-enters the AI1 single production path unchanged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContinueGoalTurn {
    pub agent_name: String,
    pub adapter: String,
    pub goal: String,
    pub workspace: String,
    pub artifacts: String,
    pub session_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub capability_profile: String,
    pub runtime_scope: String,
    pub credential_scan_policy: String,
    pub raw_prompt_policy: String,
    pub raw_output_policy: String,
    pub tool_wrapper_policy: String,
    pub live_provider_opt_in: bool,
    pub live_execution_opt_in: bool,
    pub mock_runtime_opt_in: bool,
    pub mock_provider_output_name: Option<String>,
    pub mock_provider_output_jsonl: Option<String>,
    pub timeout_seconds: u64,
    pub max_turns: u32,
    pub max_token_cost: u64,
    pub turns_taken_before: u32,
    pub token_cost_before: u64,
    pub turn_token_cost: u64,
    pub unattended: bool,
}

/// GA2 (goal-orchestration GO6): the structured goal specification a `SetGoal`
/// carries. The structured fields are stored as JSON on the goal projection so a
/// goal is durable, rebuildable state -- not transcript text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalSpec {
    pub goal_id: String,
    pub objective: String,
    pub task_id: Option<String>,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub parent_goal_id: Option<String>,
    pub attempt_run_id: Option<String>,
    /// The requirements this goal must satisfy (GO3). Each seeds a
    /// requirement-ledger row at `unverified`.
    pub requirements: Vec<GoalRequirementSpec>,
    /// Structured success criteria (GO6) as JSON.
    pub success_criteria_json: String,
    /// Structured constraints (GO6) as JSON.
    pub constraints_json: String,
    /// Structured verification surface (GO6) as JSON.
    pub verification_surface_json: String,
    /// Structured budget (GO6) as JSON.
    pub budget_json: String,
    /// Structured stop conditions (GO6) as JSON.
    pub stop_conditions_json: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalRequirementSpec {
    pub requirement_id: String,
    pub summary: String,
}

/// GA2 (goal-orchestration GO3): a per-requirement status transition record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequirementStatusRecord {
    pub requirement_id: String,
    pub goal_id: String,
    pub summary: String,
    /// `unverified` / `supported` / `validated` / `reviewed` / `blocked` /
    /// `contradicted` (GO9 requirement states).
    pub status: String,
    /// `agent_reported` (a claim) or an observed source. A `validated`/`reviewed`
    /// status backed only by `agent_reported` is rejected.
    pub source: String,
}

/// GA2 (goal-orchestration GO4): a report / evidence / review / validation
/// record against a goal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalReportRecord {
    pub goal_report_id: String,
    pub goal_id: String,
    pub session_id: Option<String>,
    pub requirement_id: Option<String>,
    /// The reporting tool / observed-evidence kind (e.g. `capo.report_progress`,
    /// `capo.record_validation`, `capo.raise_blocker`, `runtime_output`).
    pub report_kind: String,
    /// `agent_reported` (a claim) or an observed source (`runtime_output` /
    /// `adapter_event`).
    pub source: String,
    /// Agent self-declared confidence (0-100) for an `agent_reported` report;
    /// `None` for observed evidence.
    pub confidence: Option<i64>,
    pub summary: String,
    /// The artifact holding the raw body, preserved as an INPUT.
    pub body_artifact_id: Option<String>,
    /// A reference to an observed `EvidenceRecorded` row this report cites.
    pub evidence_id: Option<String>,
}

/// GA2 (goal-orchestration GO10): the rendering format of a historical report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GoalReportFormat {
    Markdown,
    Json,
}

impl GoalReportFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Json => "json",
        }
    }
}

/// A single committed event as it crosses the server transport boundary (ST4).
///
/// This is the wire shape of one entry in the event tail -- the catch-up backlog
/// and each live notification carry it. It mirrors `capo_state::EventRecord`
/// (the row the append-only log stores and `events_after` reads back) so a tail
/// is a faithful forward read of the log, never a re-serialized read-model
/// snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerEvent {
    pub sequence: i64,
    pub event_id: String,
    pub kind: String,
    pub actor: String,
    pub project_id: Option<String>,
    pub task_id: Option<String>,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub payload_json: String,
    pub redaction_state: String,
}

/// The payload a frame carries once its raw body has been withheld because the
/// event is classified `ContainsSensitive`/`Unknown` (ST7). The frame still
/// crosses the boundary -- a subscriber must see that the event happened and at
/// what sequence -- but the body is a redacted reference, never the raw content.
pub const WITHHELD_PAYLOAD_PLACEHOLDER: &str = "[REDACTED:withheld]";

impl ServerEvent {
    /// Build the egress wire shape of a committed event, applying the
    /// redaction-on-emit guard (ST7) so no frame leaves the process unredacted.
    ///
    /// This is the single funnel for every `ServerEvent` that crosses the
    /// transport boundary -- the catch-up backlog (`CapoServer::subscribe`) and
    /// each live broadcast notification (`EventStream::next_batch`) both build
    /// their events here -- so the guard runs at the egress point, not only at
    /// the tool/ACI boundary where the artifact was redacted at persist time.
    pub(crate) fn from_record(record: capo_state::EventRecord) -> Self {
        let event = Self {
            sequence: record.sequence,
            event_id: record.event_id,
            kind: record.kind,
            actor: record.actor,
            project_id: record.project_id.map(|id| id.to_string()),
            task_id: record.task_id.map(|id| id.to_string()),
            agent_id: record.agent_id.map(|id| id.to_string()),
            session_id: record.session_id.map(|id| id.to_string()),
            run_id: record.run_id.map(|id| id.to_string()),
            turn_id: record.turn_id,
            item_id: record.item_id,
            payload_json: record.payload_json,
            redaction_state: record.redaction_state,
        };
        event.redacted_for_egress()
    }

    /// Apply the [`RedactionState`](capo_state::RedactionState) guard to this
    /// event's payload before it is written to any JSON-RPC notification or SSE
    /// `Event` frame (ST7).
    ///
    /// Two layers, both at the egress point:
    ///
    /// 1. **Classification guard.** An event whose stored `redaction_state` is
    ///    not a persistable-safe state (`ContainsSensitive` / `Unknown`, or any
    ///    unrecognized state) is NOT streamed with its raw body: the payload is
    ///    replaced with a redacted reference ([`WITHHELD_PAYLOAD_PLACEHOLDER`]
    ///    plus the event id and the original classification) and the egress
    ///    `redaction_state` becomes `redacted`. The frame still crosses the
    ///    boundary so a subscriber sees the event and its sequence, but never the
    ///    sensitive content.
    /// 2. **Defense-in-depth credential scan.** For an event already labeled
    ///    safe/redacted, the same `capo-runtime` credential-shape scanner the
    ///    runner uses on process output is run over the payload as a backstop, so
    ///    a secret that slipped into a `safe`-labeled payload is scrubbed before
    ///    egress rather than streamed raw. If it scrubs anything, the egress
    ///    `redaction_state` is upgraded to `redacted`.
    fn redacted_for_egress(mut self) -> Self {
        let classification = capo_state::RedactionState::from_wire(&self.redaction_state);
        let persistable = classification
            .map(capo_state::RedactionState::is_persistable_artifact)
            .unwrap_or(false);
        if !persistable {
            // ContainsSensitive / Unknown / unrecognized: withhold the raw body.
            self.payload_json = serde_json::json!({
                "redacted": true,
                "reason": WITHHELD_PAYLOAD_PLACEHOLDER,
                "event_id": self.event_id,
                "classification": self.redaction_state,
            })
            .to_string();
            self.redaction_state = capo_state::RedactionState::Redacted.as_str().to_string();
            return self;
        }
        // Safe / Redacted: backstop credential scan over the payload at egress.
        let (scanned, state) =
            capo_runtime::RedactionPolicy::new(Vec::new()).apply(self.payload_json.as_bytes());
        if state == capo_state::RedactionState::Redacted.as_str() {
            self.payload_json = String::from_utf8_lossy(&scanned).to_string();
            self.redaction_state = capo_state::RedactionState::Redacted.as_str().to_string();
        }
        self
    }
}

/// The response to a [`ServerCommand::Subscribe`] (ST4): the catch-up backlog
/// plus the watermark the live tail resumes from.
///
/// `events` are every committed event strictly after the requested
/// `from_sequence` (and matching the session filter), in sequence order.
/// `next_sequence` is the highest sequence delivered in the backlog (or the
/// requested `from_sequence` when the backlog is empty); the live tail then
/// delivers only events with a strictly greater sequence, so there is no gap and
/// no duplicate at the backlog-to-live seam.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionBacklog {
    pub session_id: Option<String>,
    pub from_sequence: i64,
    pub next_sequence: i64,
    pub events: Vec<ServerEvent>,
}

/// The wire shape of a session's multi-turn conversation thread (ST5).
///
/// This mirrors `capo_state::SessionThread` -- the read model projected from the
/// event log -- so a client renders an ordered conversation without authoring
/// turn ordering itself. `next_sequence` is the watermark a caller resumes a
/// later read (or a `Subscribe` tail) from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerThread {
    pub session_id: String,
    pub from_sequence: i64,
    pub next_sequence: i64,
    pub turns: Vec<ServerThreadTurn>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerThreadTurn {
    pub turn_id: String,
    pub status: String,
    pub first_sequence: i64,
    pub last_sequence: i64,
    pub items: Vec<ServerThreadItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerThreadItem {
    pub sequence: i64,
    pub event_id: String,
    pub kind: String,
    pub event_kind: String,
    pub item_ref: Option<String>,
    pub text: Option<String>,
    pub redaction_state: String,
}

impl ServerThread {
    pub(crate) fn from_thread(thread: capo_state::SessionThread) -> Self {
        Self {
            session_id: thread.session_id.to_string(),
            from_sequence: thread.since_sequence,
            next_sequence: thread.next_sequence,
            turns: thread
                .turns
                .into_iter()
                .map(|turn| ServerThreadTurn {
                    turn_id: turn.turn_id,
                    status: turn.status.as_str().to_string(),
                    first_sequence: turn.first_sequence,
                    last_sequence: turn.last_sequence,
                    items: turn
                        .items
                        .into_iter()
                        .map(|item| ServerThreadItem {
                            sequence: item.sequence,
                            event_id: item.event_id,
                            kind: item.kind.as_str().to_string(),
                            event_kind: item.event_kind,
                            item_ref: item.item_ref,
                            text: item.text,
                            redaction_state: item.redaction_state,
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

impl ServerCommand {
    /// Whether this command only reads the store (never appends an event or
    /// mutates a projection). Read-only commands need not be serialized behind
    /// the transport's single-writer lock, so they can run concurrently with
    /// each other and (under WAL) alongside an in-flight write. Every other
    /// command is treated as write-bearing and serialized; defaulting unknown or
    /// future variants to write-bearing keeps the single-writer guarantee safe
    /// by construction.
    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            ServerCommand::ListAgents
                | ServerCommand::AgentStatus { .. }
                | ServerCommand::Dashboard { .. }
                // Subscribe only reads the event log (catch-up backlog) and
                // registers a broadcast subscriber; it never appends an event or
                // mutates a projection, so it need not be serialized behind the
                // single-writer lock.
                | ServerCommand::Subscribe { .. }
                // ReadThread projects a read model from the event log; it is a
                // pure forward read and never appends an event or mutates a
                // projection.
                | ServerCommand::ReadThread { .. }
                // GA2: the goal read surfaces are pure forward reads over the
                // goal projections / event log and never append an event.
                | ServerCommand::ListGoals
                | ServerCommand::ViewGoal { .. }
                | ServerCommand::GoalStory { .. }
                | ServerCommand::GoalTimeline { .. }
                | ServerCommand::GoalEvidence { .. }
                | ServerCommand::GoalValidations { .. }
                | ServerCommand::GoalReviews { .. }
                | ServerCommand::GoalRisks { .. }
                | ServerCommand::GoalReport { .. }
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerResponse {
    pub request_id: String,
    pub client_id: String,
    pub actor_id: String,
    pub input_origin: ServerInputOrigin,
    pub payload: ServerResponsePayload,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServerResponsePayload {
    AgentRegistered(AgentSummary),
    TaskSent(TaskRunSummary),
    Agents(Vec<AgentSummary>),
    AgentStatus(AgentSummary),
    Dashboard(ServerDashboardSnapshot),
    SessionStarted(TaskRunSummary),
    AdapterFixtureReplayed(AdapterReplaySummary),
    DispatchPlanned(DispatchPlanSummary),
    LiveProviderPreflighted(LiveProviderPreflightSummary),
    DispatchGated(DispatchGateSummary),
    DispatchRun(DispatchRunSummary),
    /// AI1: the outcome of driving one turn through the single production
    /// orchestration path ([`crate::CapoServer::run_dispatch_turn`]): the dispatch
    /// run summary (the existing run-completion truth) PLUS the loop's
    /// `TurnFinished` annotation, so the production path observably emits the
    /// loop's outcome rather than only the raw run.
    DispatchTurn(DispatchTurnSummary),
    Recovery(RecoverySummary),
    /// The catch-up backlog for a [`ServerCommand::Subscribe`] (ST4). Live
    /// events that follow are pushed as JSON-RPC notifications
    /// (`crate::EventNotification`), not as further responses to this request.
    Subscribed(SubscriptionBacklog),
    /// A session's multi-turn conversation thread (ST5), projected from the
    /// event log for [`ServerCommand::ReadThread`].
    Thread(ServerThread),
    /// GA2: the project's goals with a concise status summary (`ListGoals`).
    Goals(Vec<GoalStatusSummary>),
    /// GA2: one goal in full (`ViewGoal` / `SetGoal` / a lifecycle mutation).
    GoalView(Box<GoalView>),
    /// GA2: the report rows behind a story / evidence / validation / review /
    /// risk read surface.
    GoalReports(GoalReportListing),
    /// GA2: a goal event timeline (`GoalTimeline`).
    GoalTimeline(GoalTimelineView),
    /// GA2: a rendered historical execution report (`GoalReport`).
    GoalReport(GoalReportRendering),
    /// AI5: the outcome of a [`ServerCommand::ContinueGoal`]: the recorded GA4
    /// decision/reason PLUS, ONLY when the decision continued and continuation was
    /// enabled, the `DispatchTurn` summary of the ONE follow-on turn driven through
    /// the production path. `dispatched` is `Some` iff exactly one turn was driven.
    ContinuationEvaluated(ContinuationEvaluatedSummary),
}

/// AI5: the wire outcome of [`ServerCommand::ContinueGoal`]. The decision/reason is
/// the recorded GA4 verdict; `dispatched` carries the driven turn's
/// [`DispatchTurnSummary`] and is `Some` exactly when the decision was `continue`
/// (which is only reachable when continuation was enabled) -- so a caller can prove
/// "continue drove one turn, every other decision drove none" from the payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContinuationEvaluatedSummary {
    pub goal_id: String,
    pub continuation_id: String,
    /// `continue` / `pause` / `block` / `budget-limit` / `no-progress-suppress`.
    pub decision: String,
    pub reason: String,
    /// The follow-on turn driven through the single production path on a `continue`
    /// decision; `None` for every non-continuing decision (no turn was driven).
    pub dispatched: Option<DispatchTurnSummary>,
}

/// GA2 (goal-orchestration GO4/GO5): the concise per-goal status the `goals`
/// read surface renders -- objective, lifecycle status, requirement counts, and
/// the current blocker. Raw structured metadata stays behind the full view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalStatusSummary {
    pub goal_id: String,
    pub objective: String,
    pub status: String,
    pub parent_goal_id: Option<String>,
    pub attempt_run_id: Option<String>,
    pub requirement_count: usize,
    /// How many requirements are in a satisfied-or-stronger state
    /// (`supported` / `validated` / `reviewed`).
    pub requirements_supported: usize,
    pub blocked_requirement_count: usize,
    pub contradicted_requirement_count: usize,
    pub report_count: usize,
    pub blocker_reason: String,
    pub updated_sequence: i64,
}

/// GA2 (goal-orchestration GO4): one goal in full, assembled from the goal,
/// requirement-ledger, story, continuation, and delegated-provider projections.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalView {
    pub summary: GoalStatusSummary,
    pub success_criteria_json: String,
    pub constraints_json: String,
    pub verification_surface_json: String,
    pub budget_json: String,
    pub stop_conditions_json: String,
    pub task_id: Option<String>,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub requirements: Vec<GoalRequirementView>,
    pub reports: Vec<GoalReportView>,
    pub continuations: Vec<GoalContinuationView>,
    pub delegated_provider_goals: Vec<DelegatedProviderGoalView>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalRequirementView {
    pub requirement_id: String,
    pub summary: String,
    pub status: String,
    pub last_status_source: String,
    /// Whether the last status was driven by observed evidence (true) or an agent
    /// claim (false). The read model never shows a requirement validated by a
    /// claim alone.
    pub observed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalReportView {
    pub goal_report_id: String,
    pub requirement_id: Option<String>,
    pub report_kind: String,
    pub source: String,
    /// Whether this row is observed evidence (vs an `agent_reported` claim).
    pub observed: bool,
    pub confidence: Option<i64>,
    pub summary: String,
    pub body_artifact_id: Option<String>,
    pub evidence_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalContinuationView {
    pub continuation_id: String,
    pub decision: String,
    pub reason: String,
    pub attempt_run_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DelegatedProviderGoalView {
    pub delegated_goal_id: String,
    pub provider_kind: String,
    pub provider_goal_ref: Option<String>,
    pub provider_state: String,
    pub source: String,
}

/// GA2: a filtered listing of a goal's report rows for one read surface (story /
/// evidence / validation / review / risk). The `surface` field names which one
/// so the renderer can title it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalReportListing {
    pub goal_id: String,
    pub surface: String,
    pub blocker_reason: String,
    pub reports: Vec<GoalReportView>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalTimelineView {
    pub goal_id: String,
    pub entries: Vec<GoalTimelineEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalTimelineEntry {
    pub sequence: i64,
    pub event_id: String,
    pub kind: String,
    pub actor: String,
    pub redaction_state: String,
}

/// GA2 (goal-orchestration GO10): a rendered historical report. The body is the
/// markdown or JSON text; `format` names which, and `degraded` flags that some
/// referenced artifact was missing or redacted so the report rendered a clear
/// placeholder rather than raw content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalReportRendering {
    pub goal_id: String,
    pub format: String,
    pub body: String,
    pub degraded: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerDashboardSnapshot {
    pub project_id: ProjectId,
    pub agent_count: usize,
    pub active_session_count: usize,
    pub agents: Vec<AgentSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentSummary {
    pub agent_id: AgentId,
    pub name: String,
    pub status: String,
    pub current_session_id: Option<SessionId>,
    pub session: Option<SessionSummary>,
}

impl AgentSummary {
    pub(crate) fn with_session(mut self, session: Option<SessionSummary>) -> Self {
        self.session = session;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSummary {
    pub session_id: SessionId,
    pub status: String,
    pub run_id: Option<RunId>,
    pub run_status: Option<String>,
    pub adapter_kind: Option<String>,
    pub current_goal: String,
    pub latest_summary: Option<String>,
    pub latest_blocker: Option<String>,
    pub latest_confidence: Option<i64>,
    pub recent_event_count: usize,
    pub evidence_count: usize,
    pub evidence_refs: Vec<String>,
    pub review_finding_count: usize,
    pub task_outcome_report_count: usize,
    pub turn_count: usize,
    pub turn_ids: Vec<String>,
    pub latest_dispatch_plan_id: Option<String>,
    pub latest_dispatch_gate_id: Option<String>,
    pub latest_dispatch_execution_id: Option<String>,
    pub dispatch_gate_status: Option<String>,
    pub dispatch_gate_reasons: Option<String>,
    pub dispatch_next_action: Option<String>,
    pub dispatch_execution_status: Option<String>,
    pub dispatch_runtime_process_ref: Option<String>,
    pub dispatch_provider_cli_execution_allowed: Option<bool>,
    pub dispatch_provider_cli_executed: Option<bool>,
    pub dispatch_credential_scan_status: Option<String>,
    pub dispatch_raw_prompt_policy: Option<String>,
    pub dispatch_raw_output_policy: Option<String>,
    pub tool_call_count: usize,
    pub tool_observation_count: usize,
    pub memory_packet_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveProviderPreflightSummary {
    pub dispatch_plan_id: String,
    pub dispatch_gate_id: String,
    pub execution_request_id: String,
    pub adapter: String,
    pub provider_kind: String,
    pub agent_name: String,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub capability_profile: String,
    pub runtime_scope: String,
    pub credential_scan_policy: String,
    pub raw_prompt_policy: String,
    pub raw_output_policy: String,
    pub tool_wrapper_policy: String,
    pub provider_cli_execution_allowed: bool,
    pub provider_cli_executed: bool,
    pub status: String,
    pub reasons: String,
    pub next_action: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchPlanSummary {
    pub dispatch_plan_id: String,
    pub prompt_source_id: String,
    pub adapter: String,
    pub agent_name: String,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub runtime_program: String,
    pub runtime_prompt_policy: String,
    pub raw_prompt_policy: String,
    pub provider_cli_executed: bool,
    pub status: String,
}

impl DispatchPlanSummary {
    pub(crate) fn from_projection(
        plan: &AdapterDispatchPlanProjection,
        prompt_source: &AdapterDispatchPromptSourceProjection,
    ) -> Self {
        Self {
            dispatch_plan_id: plan.dispatch_plan_id.clone(),
            prompt_source_id: prompt_source.prompt_source_id.clone(),
            adapter: plan.adapter_kind.clone(),
            agent_name: plan.agent_name.clone(),
            session_id: plan.session_id.clone(),
            run_id: plan.run_id.clone(),
            runtime_program: plan.runtime_program.clone(),
            runtime_prompt_policy: plan.runtime_prompt_policy.clone(),
            raw_prompt_policy: prompt_source.raw_prompt_policy.clone(),
            provider_cli_executed: plan.provider_cli_executed,
            status: plan.status.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchGateSummary {
    pub dispatch_plan_id: String,
    pub dispatch_gate_id: String,
    pub execution_request_id: String,
    pub materialization_id: String,
    pub adapter: String,
    pub provider_cli_execution_allowed: bool,
    pub provider_cli_executed: bool,
    pub status: String,
    pub reasons: String,
    pub raw_prompt_policy: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchRunSummary {
    pub dispatch_plan_id: String,
    pub dispatch_execution_id: String,
    pub adapter: String,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub provider_cli_execution_allowed: bool,
    pub provider_cli_executed: bool,
    pub status: String,
    pub runtime_process_ref: Option<String>,
    pub credential_scan_status: String,
    pub raw_prompt_policy: String,
    pub raw_output_policy: String,
    pub reason_codes: String,
    pub input_event_count: usize,
    pub appended_event_count: usize,
    pub tool_event_count: usize,
    pub summary_event_count: usize,
    pub completed_turn_count: usize,
    /// Observed post-turn provider token/cost, in provider cost units, when the
    /// provider reports it. `None` when no token source is available (phase-1
    /// mock/live has none), in which case the RTL7 ceiling accounting falls back
    /// to the caller's pre-turn estimate. The real Codex token round-trip wires
    /// this in RTL9.
    pub observed_token_cost: Option<u64>,
}

impl DispatchRunSummary {
    pub(crate) fn from_execution(
        execution: &AdapterDispatchExecutionProjection,
        input_event_count: usize,
        appended_event_count: usize,
        tool_event_count: usize,
        summary_event_count: usize,
        completed_turn_count: usize,
    ) -> Self {
        Self {
            dispatch_plan_id: execution.dispatch_plan_id.clone(),
            dispatch_execution_id: execution.dispatch_execution_id.clone(),
            adapter: execution.adapter_kind.clone(),
            session_id: execution.session_id.clone(),
            run_id: execution.run_id.clone(),
            provider_cli_execution_allowed: execution.provider_cli_execution_allowed,
            provider_cli_executed: execution.provider_cli_executed,
            status: execution.status.clone(),
            runtime_process_ref: execution.runtime_process_ref.clone(),
            credential_scan_status: execution.credential_scan_status.clone(),
            raw_prompt_policy: execution.raw_prompt_policy.clone(),
            raw_output_policy: execution.raw_output_policy.clone(),
            reason_codes: execution.reason_codes.clone(),
            input_event_count,
            appended_event_count,
            tool_event_count,
            summary_event_count,
            completed_turn_count,
            // No provider token source on the deterministic/mock/live phase-1
            // path; RTL9 wires the real Codex token round-trip here.
            observed_token_cost: None,
        }
    }
}

/// AI1: the wire-facing outcome of one turn driven through
/// [`crate::CapoServer::run_dispatch_turn`]. It pairs the dispatch run summary
/// (the single run-completion truth) with the loop's `TurnFinished` annotation
/// and the optional ceiling-breach reason, so the production operator/live-run
/// path returns the SAME loop outcome the in-process tests assert -- one path,
/// one event shape.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchTurnSummary {
    pub run: DispatchRunSummary,
    pub finished: TurnFinishedSummary,
    /// When the live turn was aborted by the resource ceiling, the stable breach
    /// code (e.g. `max_turns_exceeded`); `None` on a normal completion.
    pub ceiling_breach_code: Option<String>,
}

/// AI1: the wire shape of [`capo_controller::TurnFinished`]'s replay-stable,
/// equality-significant fields. The volatile append-count `replay` diagnostic is
/// deliberately omitted -- it is per-run and not replay-stable -- so this carries
/// only the fields a client can rely on across a restart/replay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnFinishedSummary {
    pub turn_id: String,
    /// `completed` / `interrupted` / `stopped` / `failed`.
    pub stop_reason: String,
    pub observed_terminal_event: bool,
    pub summary_refs: Vec<String>,
    pub observed_tool_refs: Vec<String>,
}

impl TurnFinishedSummary {
    pub(crate) fn from_finished(finished: &capo_controller::TurnFinished) -> Self {
        Self {
            turn_id: finished.turn_id.to_string(),
            stop_reason: finished.stop_reason.as_str().to_string(),
            observed_terminal_event: finished.observed_terminal_event,
            summary_refs: finished.summary_refs.clone(),
            observed_tool_refs: finished.observed_tool_refs.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskRunSummary {
    pub task_id: TaskId,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub runtime_process_ref: String,
    pub external_session_ref: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterReplaySummary {
    pub adapter: String,
    pub fixture_name: String,
    pub fixture_hash: String,
    pub agent_name: String,
    pub task_id: TaskId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub turn_id: String,
    pub provider_cli_executed: bool,
    pub raw_content_policy: String,
    pub input_event_count: usize,
    pub appended_event_count: usize,
    pub tool_event_count: usize,
    pub summary_event_count: usize,
    pub completed_turn_count: usize,
}

impl TaskRunSummary {
    pub(crate) fn from_run_refs(run: FakeRunRefs) -> Self {
        Self {
            task_id: run.task_id,
            agent_id: run.agent_id,
            session_id: run.session_id,
            run_id: run.run_id,
            runtime_process_ref: run.runtime_process_ref,
            external_session_ref: run.external_session_ref,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoverySummary {
    pub recovery_attempt_id: String,
    pub recovered_run_count: usize,
    pub watermark: Option<i64>,
}

fn default_request_id(command: &ServerCommand) -> String {
    match command {
        ServerCommand::RegisterAgent { name, .. } => {
            format!("server-agent-register-{}", slug(name))
        }
        ServerCommand::SendTask {
            agent_name, goal, ..
        } => {
            format!("server-task-send-{}-{}", slug(agent_name), slug(goal))
        }
        ServerCommand::SteerAgent { agent_name, goal } => {
            format!(
                "server-agent-steer-{}-{}",
                slug(agent_name),
                stable_hash(goal.as_bytes())
            )
        }
        ServerCommand::InterruptAgent { agent_name, reason } => {
            format!(
                "server-agent-interrupt-{}-{}",
                slug(agent_name),
                stable_hash(reason.as_bytes())
            )
        }
        ServerCommand::StopAgent { agent_name, reason } => {
            format!(
                "server-agent-stop-{}-{}",
                slug(agent_name),
                stable_hash(reason.as_bytes())
            )
        }
        ServerCommand::ListAgents => "server-agent-list".to_string(),
        ServerCommand::AgentStatus { agent_name } => {
            format!("server-agent-status-{}", slug(agent_name))
        }
        ServerCommand::Dashboard { .. } => "server-dashboard".to_string(),
        ServerCommand::StartSession {
            adapter,
            agent_name,
            goal,
            session_id,
            run_id,
            ..
        } => {
            format!(
                "server-session-start-{}-{}-{}-{}-{}",
                slug(adapter),
                slug(agent_name),
                session_id
                    .as_deref()
                    .map(slug)
                    .unwrap_or_else(|| "auto-session".to_string()),
                run_id
                    .as_deref()
                    .map(slug)
                    .unwrap_or_else(|| "auto-run".to_string()),
                stable_hash(goal.as_bytes())
            )
        }
        ServerCommand::ReplayAdapterFixture {
            adapter,
            session_id,
            run_id,
            turn_id,
            fixture_name,
            ..
        } => {
            format!(
                "server-adapter-replay-{}-{}-{}-{}-{}",
                slug(adapter),
                slug(session_id),
                slug(run_id),
                slug(turn_id),
                slug(fixture_name)
            )
        }
        ServerCommand::PlanDispatch {
            adapter,
            agent_name,
            goal,
            ..
        } => {
            format!(
                "server-dispatch-plan-{}-{}-{}",
                slug(adapter),
                slug(agent_name),
                slug(goal)
            )
        }
        ServerCommand::PreflightLiveProvider {
            adapter,
            agent_name,
            goal,
            session_id,
            run_id,
            turn_id,
            ..
        } => {
            format!(
                "server-dispatch-live-preflight-{}-{}-{}-{}-{}-{}",
                slug(adapter),
                slug(agent_name),
                slug(session_id),
                slug(run_id),
                slug(turn_id),
                stable_hash(goal.as_bytes())
            )
        }
        ServerCommand::GateDispatch { dispatch_plan_id } => {
            format!("server-dispatch-gate-{}", slug(dispatch_plan_id))
        }
        ServerCommand::RunDispatchLocal {
            dispatch_plan_id,
            fixture_name,
            ..
        } => {
            format!(
                "server-dispatch-run-local-{}-{}",
                slug(dispatch_plan_id),
                slug(fixture_name)
            )
        }
        ServerCommand::RunLiveProviderLocal {
            dispatch_plan_id,
            goal,
            mock_provider_output_name,
            ..
        } => {
            format!(
                "server-dispatch-live-run-local-{}-{}-{}",
                slug(dispatch_plan_id),
                stable_hash(goal.as_bytes()),
                mock_provider_output_name
                    .as_deref()
                    .map(slug)
                    .unwrap_or_else(|| "provider".to_string())
            )
        }
        ServerCommand::RunDispatchTurn {
            adapter,
            agent_name,
            goal,
            session_id,
            run_id,
            turn_id,
            ..
        } => {
            format!(
                "server-dispatch-turn-{}-{}-{}-{}-{}-{}",
                slug(adapter),
                slug(agent_name),
                slug(session_id),
                slug(run_id),
                slug(turn_id),
                stable_hash(goal.as_bytes())
            )
        }
        ServerCommand::Recover => "server-recover".to_string(),
        ServerCommand::Subscribe {
            session_id,
            from_sequence,
        } => format!(
            "server-subscribe-{}-{}",
            session_id
                .as_deref()
                .map(slug)
                .unwrap_or_else(|| "all".to_string()),
            from_sequence
        ),
        ServerCommand::ReadThread {
            session_id,
            from_sequence,
        } => format!("server-thread-{}-{}", slug(session_id), from_sequence),
        ServerCommand::SetGoal { spec } => format!("server-goal-set-{}", slug(&spec.goal_id)),
        ServerCommand::PauseGoal { goal_id } => format!("server-goal-pause-{}", slug(goal_id)),
        ServerCommand::ResumeGoal { goal_id } => format!("server-goal-resume-{}", slug(goal_id)),
        ServerCommand::BlockGoal { goal_id, reason } => format!(
            "server-goal-block-{}-{}",
            slug(goal_id),
            stable_hash(reason.as_bytes())
        ),
        ServerCommand::ClearGoal { goal_id, reason } => format!(
            "server-goal-clear-{}-{}",
            slug(goal_id),
            stable_hash(reason.as_bytes())
        ),
        ServerCommand::SetRequirementStatus { record } => format!(
            "server-goal-requirement-{}-{}",
            slug(&record.requirement_id),
            slug(&record.status)
        ),
        ServerCommand::RecordGoalReport { report } => {
            format!("server-goal-report-{}", slug(&report.goal_report_id))
        }
        ServerCommand::MarkGoalComplete { goal_id } => {
            format!("server-goal-mark-complete-{}", slug(goal_id))
        }
        ServerCommand::ListGoals => "server-goals-list".to_string(),
        ServerCommand::ViewGoal { goal_id } => format!("server-goal-view-{}", slug(goal_id)),
        ServerCommand::GoalStory { goal_id } => format!("server-goal-story-{}", slug(goal_id)),
        ServerCommand::GoalTimeline { goal_id } => {
            format!("server-goal-timeline-{}", slug(goal_id))
        }
        ServerCommand::GoalEvidence { goal_id } => {
            format!("server-goal-evidence-{}", slug(goal_id))
        }
        ServerCommand::GoalValidations { goal_id } => {
            format!("server-goal-validations-{}", slug(goal_id))
        }
        ServerCommand::GoalReviews { goal_id } => format!("server-goal-reviews-{}", slug(goal_id)),
        ServerCommand::GoalRisks { goal_id } => format!("server-goal-risks-{}", slug(goal_id)),
        ServerCommand::GoalReport { goal_id, format } => {
            format!("server-goal-report-{}-{}", slug(goal_id), format.as_str())
        }
        ServerCommand::ContinueGoal {
            goal_id,
            continuation_id,
            ..
        } => format!(
            "server-goal-continue-{}-{}",
            slug(goal_id),
            slug(continuation_id)
        ),
    }
}
