//! The run driver (Faz 4-b/4-c): FSM phases (§6.1), the DAG timer (§6.2),
//! gates (§9) and the §7.4 operation ledger, glued around the §7.3 event log.
//!
//! Synchronous on purpose: `Scheduling` is deterministic and single-writer
//! (§7.5); async inference arrives with the real agents (Faz 5), which plug
//! in through the [`TaskExecutor`] seam. The golden mini-run (§22 Faz 4
//! çıkış kriteri) proves: 3 fake tasks, one parallel group, kill→resume with
//! the SAME final hash, and every §7.4-B4 discrete-crash window.
//!
//! The §7.4 v2.47 write order is ONE path — ledger `started` → state.db
//! transaction → events.jsonl append → ledger `finished` — so the atomicity
//! test can arm a kill after each step; nothing else completes a task.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::params;
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::events::EventLog;
use crate::fsm::{self, FsmError, PauseReason, Phase, RunState, Trigger};
use crate::gates::{self, ApprovalRow, Gate, GateOutcome, GrantedReason};
use crate::hash;
use crate::ledger::{self, kind as lk, Ledger, ResumeAction};
use crate::models::event::EventKind as Ev;
use crate::models::task::{TaskItem, TaskPlan};
use crate::scheduler::{self, Scheduler, Status};
use crate::state::StateStore;

/// One injectable crash window of the §7.4-B4 write order, plus the work
/// step (between `started` and the first transaction). `AfterFinish` models
/// "no crash" — resume then finds nothing to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    AfterStarted,
    AfterWork,
    AfterDoneTx,
    AfterDoneEvent,
    AfterFinish,
}

/// What a task implementation returns once its work is done (§5.3 output).
#[derive(Debug, Clone)]
pub struct Executed {
    /// Serialized into the result file AND the TaskCompleted event.
    pub summary: Value,
    /// false → the ladder (§8) owns this task; the mini driver fails the run.
    pub ok: bool,
    /// Real spend of this execution in USD (§8.2 reconciliation). Faz 5 fills
    /// it from the EnvLayer `actual_cost_usd`; 0.0 for a no-LLM task. Tracked
    /// so `runs.spent_usd` is live, not a decorative column.
    pub cost_usd: f64,
    /// §8.2/B3: the execution was refused at the CALL boundary for money
    /// reasons (EnvLayer `ModelBudgetDenied`). The task is NOT defective —
    /// the driver pauses the run (completed work kept, this task stays
    /// pending) instead of failing it into the ladder.
    pub budget_denied: bool,
}

/// The seam Faz 5 fills with real agents (LLM + ToolGateway behind
/// SchemaGuard); the golden test fills it with a deterministic fake.
pub trait TaskExecutor {
    fn execute(&self, task: &TaskItem, op_id: &str) -> Result<Executed>;
}

/// The Faz-4 CLI placeholder executor (§22): records the task as shipped
/// without any LLM/tool call, so `ribozym run --mode mini` is fully offline
/// and deterministic. Replaced by the A-agent pipeline in Faz 5.
pub struct MockExecutor;
impl TaskExecutor for MockExecutor {
    fn execute(&self, task: &TaskItem, _op_id: &str) -> Result<Executed> {
        Ok(Executed {
            summary: json!({"task": task.id, "agent": task.assigned_agent, "mock": true}),
            ok: true,
            // A mock makes no API call: honest cost is zero, so the mini-run
            // exercises the accounting PIPELINE without pretending to spend.
            cost_usd: 0.0,
            budget_denied: false,
        })
    }
}

/// Everything the driver needs to know about the world; kept flat and
/// injectable (epoch clock!) so the golden test is reproducible.
#[derive(Debug, Clone)]
pub struct RunConfig {
    /// Working directory holding `.ribozym/` and `runs/` (a project repo root).
    pub root: PathBuf,
    pub spec_id: String,
    pub run_id: String,
    pub autonomy_level: u8,
    pub batch_yes: bool,
    pub max_concurrency: u32,
    pub crisis_max: u32,
    pub gate_timeout_seconds: u64,
    pub budget_usd: f64,
    /// §8.2 soft/hard thresholds as percentages OF `budget_usd`. Defaults come
    /// from `[budget]` in ribozym.toml; the driver clamps spend against them
    /// (soft → one-time BudgetWarning, hard → refuse the next call → Paused).
    pub budget_soft_pct: f64,
    pub budget_hard_pct: f64,
    /// Injectable clock (gate timeouts + audit stamps) — no wall-clock races.
    pub now_epoch: u64,
}

/// The Finalizing/FinalReview seam (Faz 5-c): the driver stays synchronous
/// and single-writer; A3's report and A1's referee verdict plug in HERE, at
/// exactly the two phase boundaries §6.1 reserves for them. `advance` keeps
/// its bool signature for the deterministic mini path via [`FixedCompliance`].
pub trait FinalPipeline {
    /// `Finalizing` (all tasks terminal, before `ReportReady`): produce the
    /// run report (A3) — numbers from the system of record, never the model.
    fn finalize(&mut self, o: &mut Orchestrator) -> Result<()>;
    /// `FinalReview`: A1's adjudicated verdict. Returning true still cannot
    /// deliver over a non-done task set (the scheduler's truth wins, P2).
    fn review(&mut self, o: &mut Orchestrator) -> Result<bool>;
    /// `Crisis` (E2, §5.6 m5): the CEO may re-cut unfinished work IN SCOPE
    /// before the run returns to FinalReview. The default is honest absence
    /// of a replanner (mini path / Faz-4 behavior): the crisis loop
    /// re-enters FinalReview unchanged and the cap escalates.
    fn replan(&mut self, o: &mut Orchestrator) -> Result<ReplanOutcome> {
        let _ = o;
        Ok(ReplanOutcome::NoChange)
    }
}

/// What [`FinalPipeline::replan`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplanOutcome {
    /// The plan was revised AND absorbed — re-enter Scheduling so the new
    /// work actually runs (FSM Trigger::Replanned).
    Revised,
    /// Nothing changed (no replanner, or the CEO found no in-scope cut) —
    /// straight back to FinalReview; the crisis cap keeps the loop finite.
    NoChange,
}

/// The mini path's verdict: a fixed bool, no report generation — byte-identical
/// behavior to the pre-Faz-5-c `advance` (golden tests pin this).
pub struct FixedCompliance(pub bool);
impl FinalPipeline for FixedCompliance {
    fn finalize(&mut self, _o: &mut Orchestrator) -> Result<()> {
        Ok(())
    }
    fn review(&mut self, _o: &mut Orchestrator) -> Result<bool> {
        Ok(self.0)
    }
}

/// The pre-scheduling seam (Faz 5-d): the §6.1 management line from a user
/// form to a validated task plan. Same shape as [`FinalPipeline`] — the
/// driver stays synchronous and single-writer; A1's draft and A2's plan plug
/// in at exactly the phase boundaries §6.1 reserves for them, with the REAL
/// form check, the REAL spec gate and ledger-guarded turns underneath.
pub trait PlanningPipeline {
    /// `Idle` → `SpecSubmitted`: persist the user's form bytes as the run's
    /// `raw_input.md` — the provenance file `source.raw_input_path` names.
    fn save_form(&mut self, o: &mut Orchestrator) -> Result<()>;
    /// `Researching`: A1's one big call; lands as `spec.json` on disk.
    fn draft_spec(&mut self, o: &mut Orchestrator) -> Result<()>;
    /// `Advising`: the non-interactive fast path locks the decisions as they
    /// stand. `false` parks the run IN Advising (no transition — resume
    /// re-locks after the form improves); an under-specified spec is never
    /// silently continued (§5.4C silent-guess ban).
    fn lock_decisions(&mut self, o: &mut Orchestrator) -> Result<bool>;
    /// `SpecValidated` (spec gate passed): A2's plan — persisted to
    /// `plan.json` and returned for the driver to register as task rows.
    fn plan_tasks(&mut self, o: &mut Orchestrator) -> Result<TaskPlan>;
}

/// The caller-facing result of [`Orchestrator::advance_planning`]. `Planned`
/// hands over to [`advance_with`] at Scheduling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningOutcome {
    Planned,
    Waiting {
        gate: Gate,
    },
    NeedsUser {
        why: String,
    },
    /// §8.2/B3: a planning turn hit the call-boundary wall — the run parks
    /// `Paused(budget)` with `paused_from` recorded; raise the budget and
    /// resume re-runs the SAME phase (the unfinished op re-derives its id).
    Paused {
        reason: crate::fsm::PauseReason,
    },
}

/// The caller-facing result of [`Orchestrator::advance`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Delivered,
    /// Terminal: a veto, a delivery park, or exhausted replans.
    NeedsUser {
        why: String,
    },
    Failed {
        why: String,
    },
    /// A gate waits for a human (or its low-risk timeout): nothing is lost —
    /// `resume` re-checks with a later clock value / fresh approval row.
    Waiting {
        gate: Gate,
    },
    /// §8.2/B3 recoverable wall: the next task's estimated cost would breach
    /// the hard budget, so it was NOT started (ledger untouched, finished work
    /// kept). Raise `budget_usd` and `resume` to continue from `paused_from`.
    Paused {
        reason: PauseReason,
    },
}

pub struct Orchestrator {
    cfg: RunConfig,
    store: StateStore,
    events: EventLog,
    by_id: BTreeMap<String, TaskItem>,
    sched: Scheduler,
    run: RunState,
    /// Live mirror of `runs.spent_usd` — the §8.2 clamp reads this, commit
    /// writes it (persisted in the same transaction as the task state).
    spent_usd: f64,
    budget_warned: bool,
    /// Armed once: the first hit crashes (models a single kill per resume).
    crash_at: Cell<Option<Stage>>,
    /// Set when an execution was refused at the call boundary for money
    /// reasons (§8.2/B3): the Scheduling batch finishes, then the driver
    /// pauses the run instead of failing work that is merely unaffordable.
    budget_paused: bool,
}

// `store`/`events` hold OS handles that are not Debug, so project only the
// control-plane state (useful for `.unwrap_err()` diagnostics in tests).
impl std::fmt::Debug for Orchestrator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Orchestrator")
            .field("spec_id", &self.cfg.spec_id)
            .field("run", &self.run)
            .field("spent_usd", &self.spent_usd)
            .field("budget_warned", &self.budget_warned)
            .field("statuses", &self.sched.statuses())
            .finish()
    }
}

impl Orchestrator {
    // ---- construction / resume (§7.4) ----------------------------------

    /// True when `runs` already holds this spec — the CLI's `run` command
    /// resumes instead of creating.
    pub fn exists(cfg: &RunConfig) -> Result<bool> {
        let db = cfg.root.join(".ribozym").join("state.db");
        if !db.exists() {
            return Ok(false);
        }
        let store = StateStore::open(&db)?;
        let n: i64 = store.conn().query_row(
            "SELECT COUNT(*) FROM runs WHERE spec_id = ?1",
            params![cfg.spec_id],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// Fresh run: persists `runs` at `tasks_created` + all `tasks` pending.
    /// No spec ⇒ the §6.4 cross-doc traceability gate cannot run (it needs the
    /// requirement set); the CLI wires the checked variant with a spec.
    pub fn create(cfg: &RunConfig, plan: &TaskPlan) -> Result<Self> {
        Self::create_with_spec(cfg, plan, None)
    }

    /// Fresh run starting at the §6.1 BEGINNING: `runs.status = 'idle'`, no
    /// task rows yet. The planning lifecycle (`advance_planning`) drives the
    /// form gate, the A1 draft, the spec gate and the A2 plan from here; the
    /// mini path (`create`/`create_with_spec`) starts at `tasks_created`.
    pub fn create_idle(cfg: &RunConfig) -> Result<Self> {
        let store = StateStore::open(&cfg.root.join(".ribozym").join("state.db"))?;
        {
            let conn = store.conn();
            conn.execute(
                "INSERT OR REPLACE INTO runs(spec_id, run_id, status, autonomy_level, budget_usd, created_at, updated_at)
                 VALUES (?1, ?2, 'idle', ?3, ?4, ?5, ?5)",
                params![cfg.spec_id, cfg.run_id, cfg.autonomy_level as i64, cfg.budget_usd, stamp(cfg)],
            )?;
        }
        Self::build(cfg, store, &TaskPlan::default())
    }

    /// `create` + §6.4 enforcement: when a `Spec` is supplied, the task graph
    /// must cover every requirement (no `TASK_UNCOVERED_REQUIREMENT`) before
    /// `TasksCreated` is allowed. A rejection here means NO run/tasks rows are
    /// written — the bad plan is refused at the gate, not half-persisted.
    pub fn create_with_spec(
        cfg: &RunConfig,
        plan: &TaskPlan,
        spec: Option<&crate::models::spec::Spec>,
    ) -> Result<Self> {
        if let Some(spec) = spec {
            let problems = crate::validate::check_traceability(spec, plan);
            if !problems.is_empty() {
                return Err(Error::Traceability(problems.join("; ")));
            }
        }
        let store = StateStore::open(&cfg.root.join(".ribozym").join("state.db"))?;
        {
            let conn = store.conn();
            conn.execute(
                "INSERT OR REPLACE INTO runs(spec_id, run_id, status, autonomy_level, budget_usd, created_at, updated_at)
                 VALUES (?1, ?2, 'tasks_created', ?3, ?4, ?5, ?5)",
                params![cfg.spec_id, cfg.run_id, cfg.autonomy_level as i64, cfg.budget_usd, stamp(cfg)],
            )?;
            for t in &plan.tasks {
                conn.execute(
                    "INSERT OR REPLACE INTO tasks(spec_id, task_id, status, agent, retry_count, escalation_level)
                     VALUES (?1, ?2, 'pending', ?3, 0, 0)",
                    params![cfg.spec_id, t.id, t.assigned_agent],
                )?;
            }
        }
        let mut me = Self::build(cfg, store, plan)?;
        me.events.append_with(
            Ev::TasksCreated,
            "op-plan",
            None,
            &[("total", json!(plan.tasks.len()))],
        )?;
        Ok(me)
    }

    /// Resume (§7.4): reopen, recover unfinished ledger ops, re-sync the FSM
    /// and the scheduler from persisted truth, hand back a ready driver.
    pub fn resume(cfg: &RunConfig, plan: &TaskPlan) -> Result<Self> {
        let store = StateStore::open(&cfg.root.join(".ribozym").join("state.db"))?;
        let mut me = Self::build(cfg, store, plan)?;
        me.recover_ledger()?;
        Ok(me)
    }

    fn build(cfg: &RunConfig, store: StateStore, plan: &TaskPlan) -> Result<Self> {
        let mut by_id = BTreeMap::new();
        for t in &plan.tasks {
            by_id.insert(t.id.clone(), t.clone());
        }
        let events = EventLog::open(
            &run_dir(&cfg.root, &cfg.spec_id).join("events.jsonl"),
            &cfg.spec_id,
            &cfg.run_id,
        )?;
        // §7.4 step 3: FSM syncs to the persisted phase; the scheduler to
        // the persisted task statuses. Pause anchors + budget come from the
        // same row so a `Paused(budget)` run rebuilds truthfully.
        let row: (String, i64, f64, i64, Option<String>, Option<String>) = store.conn().query_row(
            "SELECT status, crisis_count, spent_usd, budget_warned, paused_from, paused_reason
             FROM runs WHERE spec_id = ?1",
            params![cfg.spec_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )?;
        let (status, crisis, spent_usd, warned, paused_from_s, paused_reason_s) = row;
        let phase = phase_of_str(&status).ok_or_else(|| {
            Error::ConfigInvalid(format!("runs.status {status:?} is not a known phase"))
        })?;
        // A persisted pause must carry BOTH the phase and the reason; a
        // `paused` status with a lost anchor is corrupt and must not guess.
        let (paused, paused_from) = if phase == Phase::Paused {
            let reason = match paused_reason_s.as_deref() {
                Some("budget") => Some(crate::fsm::PauseReason::Budget),
                Some("checkpoint") => Some(crate::fsm::PauseReason::Checkpoint),
                other => {
                    return Err(Error::ConfigInvalid(format!(
                        "runs.paused_reason {other:?} invalid for a paused run"
                    )))
                }
            };
            let from = paused_from_s
                .as_deref()
                .and_then(phase_of_str)
                .ok_or_else(|| Error::ConfigInvalid("paused run missing paused_from".into()))?;
            (reason, Some(from))
        } else {
            (None, None)
        };
        let mut sched = Scheduler::from_plan(plan, cfg.max_concurrency)
            .map_err(|e| Error::Scheduler(e.to_string()))?;
        // Resume honesty: task rows without a plan document mean the caller
        // lost the plan (planning keeps it at runs/<spec>/plan.json) — an
        // empty-plan resume over rows would rebuild a scheduler that
        // disagrees with the persisted task set (§7.1). Refuse, don't guess.
        let row_count: i64 = store.conn().query_row(
            "SELECT COUNT(*) FROM tasks WHERE spec_id = ?1",
            params![cfg.spec_id],
            |r| r.get(0),
        )?;
        if plan.tasks.is_empty() && row_count > 0 {
            return Err(Error::ConfigInvalid(
                "tasks rows exist but no plan was supplied — resume needs runs/<spec>/plan.json \
                 once the run is past TasksCreated"
                    .into(),
            ));
        }
        for (id, st) in task_rows(store.conn(), &cfg.spec_id)? {
            sched.set_status(&id, st);
        }
        Ok(Self {
            cfg: cfg.clone(),
            store,
            events,
            by_id,
            sched,
            run: RunState {
                phase,
                crisis_count: crisis as u32,
                paused,
                paused_from,
            },
            spent_usd,
            budget_warned: warned != 0,
            crash_at: Cell::new(None),
            budget_paused: false,
        })
    }

    /// Arm a kill for the NEXT time the driver reaches that write step.
    pub fn arm_crash(&self, stage: Stage) {
        self.crash_at.set(Some(stage));
    }

    fn hit(&self, stage: Stage) -> Result<()> {
        if self.crash_at.get() == Some(stage) {
            self.crash_at.set(None); // one-shot: the resumed driver runs clean
            return Err(Error::Crashed(stage));
        }
        Ok(())
    }

    /// §7.4 steps 1–2: close every unfinished ledger operation. The ONLY
    /// thing resume does before the scheduler continues.
    fn recover_ledger(&mut self) -> Result<()> {
        let ops = Ledger::new(self.store.conn()).unfinished(&self.cfg.spec_id)?;
        for op in ops {
            match ledger::plan_resume(&op) {
                ResumeAction::FinalizeResult => {
                    // "iş bitmiştir, yalnız yazma kesilmiştir → sonuç tekrar
                    // yazılır": the summary AND the cost come from the
                    // persisted envelope, the executor is NEVER re-invoked.
                    let (summary, cost) = self.read_result(&op)?;
                    if op.kind == ledger::kind::PLAN_TURN {
                        // A planning turn belongs to no DAG task: no task
                        // rows, no task event — only the charge-guarded spend
                        // and the close; the phase loop re-derives the
                        // document from the finished op (§7.4 rule 4).
                        self.commit_plan_op(&op.op_id, cost)?;
                    } else {
                        self.commit_task(&op.op_id, op.task_id.as_deref(), &summary, true, cost)?;
                    }
                }
                ResumeAction::ReRun => {
                    // Same payload → same op_id → the INSERT in the normal
                    // path is idempotent. An in_progress row from before the
                    // kill is rolled back to pending so the scheduler
                    // re-admits it (a fresh attempt on identical input is
                    // explicitly allowed by §7.4).
                    if let Some(tid) = op.task_id.as_deref() {
                        if self.sched.status(tid) == Some(Status::InProgress) {
                            self.sched.set_status(tid, Status::Pending);
                            self.write_task_row(tid, Status::Pending, 0)?;
                        }
                    }
                }
                ResumeAction::Done => {}
            }
        }
        // Re-sync the scheduler after recovery rewrote rows.
        for (id, st) in task_rows(self.store.conn(), &self.cfg.spec_id)? {
            self.sched.set_status(&id, st);
        }
        Ok(())
    }

    // ---- the phase loop ---------------------------------------------------

    /// Drive the FSM to a terminal outcome, running scheduled work through
    /// `exec`. `compliance_ok` is the FinalReview verdict (§6.1) — the
    /// Faz-4 mini path's fixed-bool form of [`advance_with`].
    pub fn advance(&mut self, exec: &dyn TaskExecutor, compliance_ok: bool) -> Result<Outcome> {
        self.advance_with(exec, &mut FixedCompliance(compliance_ok))
    }

    /// [`advance`] with the real Faz-5 pipeline: A3 reports at `Finalizing`,
    /// A1 referees at every `FinalReview` pass (including crisis re-reviews).
    pub fn advance_with(
        &mut self,
        exec: &dyn TaskExecutor,
        pipe: &mut dyn FinalPipeline,
    ) -> Result<Outcome> {
        loop {
            match self.run.phase {
                Phase::TasksCreated => match self.pass_gate(Gate::TaskPlan)? {
                    GateStep::Passed => self.fire(Trigger::TaskGraphValidated)?,
                    GateStep::Waiting => return Ok(Outcome::Waiting { gate: Gate::TaskPlan }),
                    GateStep::Vetoed => return Ok(self.vetoed(Gate::TaskPlan)),
                },
                Phase::Scheduling => {
                    if self.sched.all_terminal() {
                        self.fire(Trigger::AllTasksTerminal)?;
                        continue;
                    }
                    if self.sched.stalled() {
                        // §6.2: the Finalizing signal needs ALL terminal AND
                        // ≥1 done — nothing deliverable. Terminality must
                        // also reach `runs.status` (§6.1 Failed is terminal;
                        // leaving the row at `scheduling` would let a resume
                        // re-enter the dead loop).
                        self.fire(Trigger::FatalError)?;
                        return Ok(Outcome::Failed {
                            why: "no task can reach `done` (all failed/blocked)".into(),
                        });
                    }
                    let admission = self.sched.admit();
                    for (t, with) in &admission.deferred_conflict {
                        self.events.append_with(
                            Ev::TaskOwnershipConflict,
                            "op-sched",
                            Some(t),
                            &[("waits_for", json!(with))],
                        )?;
                    }
                    // §8.2 run-level pre-clamp: reserve each task's DECLARED
                    // estimate against the hard wall before starting it. Only
                    // estimates are honored — we never invent a cost for an
                    // un-estimated task ("tahmin uydurmaz"); unpriceable work
                    // is stopped at the call boundary by EnvLayer instead
                    // (ModelBudgetDenied → ladder §8, never silent overspend).
                    let hard_wall = self.cfg.budget_usd * self.cfg.budget_hard_pct / 100.0;
                    let mut projected = self.spent_usd;
                    let mut paused = false;
                    let mut begin_ids: Vec<String> = Vec::new();
                    for id in &admission.admitted {
                        let est = self
                            .by_id
                            .get(id)
                            .and_then(|t| t.estimated.as_ref())
                            .map(|e| e.cost_usd)
                            .unwrap_or(0.0);
                        if self.cfg.budget_usd > 0.0 && projected + est > hard_wall + f64::EPSILON {
                            paused = true;
                            break; // the rest waits; completed work is kept
                        }
                        projected += est;
                        begin_ids.push(id.clone());
                    }
                    if paused && begin_ids.is_empty() {
                        self.fire(Trigger::BudgetHardLimit)?;
                        return Ok(Outcome::Paused {
                            reason: crate::fsm::PauseReason::Budget,
                        });
                    }
                    for id in &begin_ids {
                        // Begin the WHOLE group first: every member carries
                        // its `TaskStarted` before any member completes —
                        // the parallel group must be visible in the event
                        // stream, not only in the scheduler's slot math.
                        // op_id is derived (pure §7.4 identity), so the event
                        // already correlates to the ledger row to come.
                        let task = self.by_id[id].clone();
                        let payload = scheduler::task_payload(&task);
                        let op =
                            Ledger::op_id(lk::AGENT_CALL, &self.cfg.spec_id, Some(id), &payload);
                        self.sched.begin(id);
                        self.write_task_row(id, Status::InProgress, 0)?;
                        self.events.append_with(
                            Ev::TaskStarted,
                            &op,
                            Some(id),
                            &[("agent", json!(task.assigned_agent))],
                        )?;
                    }
                    for id in begin_ids {
                        self.run_one_task(&id, exec)?;
                    }
                    if self.budget_paused {
                        // A call boundary refused for money: pause honestly —
                        // the denied task is pending again, finished work
                        // stays finished, budget raise + resume continues.
                        self.budget_paused = false;
                        self.fire(Trigger::BudgetHardLimit)?;
                        return Ok(Outcome::Paused {
                            reason: crate::fsm::PauseReason::Budget,
                        });
                    }
                    // Partial admission under the wall: run the fitting
                    // prefix, then pause so the remainder resumes when the
                    // budget is raised (next loop iteration re-clamps).
                    if paused {
                        self.fire(Trigger::BudgetHardLimit)?;
                        return Ok(Outcome::Paused {
                            reason: crate::fsm::PauseReason::Budget,
                        });
                    }
                }
                Phase::Finalizing => {
                    // §5.7 m4: A3's report is produced BEFORE the delivery
                    // gate. The mini path's FixedCompliance no-ops here —
                    // golden digests stay byte-identical to Faz 4.
                    pipe.finalize(self)?;
                    self.fire(Trigger::ReportReady)?;
                }
                Phase::FinalReview => {
                    let compliance_ok = pipe.review(self)?;
                    // §6.1's spec_match_pct can be 100 only when the work it
                    // summarizes ACTUALLY succeeded: a run whose task set
                    // contains a non-done terminal (failed/blocked/skipped)
                    // may not deliver on a blanket "compliant" verdict — that
                    // would be a fake green light over red work. A genuine
                    // compliance verdict is the caller's; task truth is the
                    // scheduler's, and the scheduler wins.
                    if !self.sched.statuses().values().all(|(s, _)| *s == Status::Done)
                        && compliance_ok
                    {
                        tracing::warn!(
                            "FinalReview: tasks not all done but compliance claimed — forcing ComplianceFailed"
                        );
                        self.fire(Trigger::ComplianceFailed)?;
                        continue;
                    }
                    if !compliance_ok {
                        self.fire(Trigger::ComplianceFailed)?;
                        continue;
                    }
                    match self.pass_gate(Gate::Delivery)? {
                        GateStep::Passed => self.fire(Trigger::ComplianceMet)?,
                        GateStep::Waiting => return Ok(Outcome::Waiting { gate: Gate::Delivery }),
                        GateStep::Vetoed => return Ok(self.vetoed(Gate::Delivery)),
                    }
                }
                Phase::Crisis => {
                    // §5.6 m5: a real replanner (Faz 5-d pipeline) re-cuts
                    // the unfinished work and the run re-enters Scheduling;
                    // the default hook keeps the Faz-4 no-op behavior.
                    match pipe.replan(self)? {
                        ReplanOutcome::Revised => self.fire(Trigger::Replanned)?,
                        ReplanOutcome::NoChange => self.fire(Trigger::ReplanComplete)?,
                    }
                }
                Phase::Paused => {
                    // The wall is durable: advancing a paused run without
                    // raising the budget must NOT silently continue (§8.2/B3).
                    // `unpause()` (called after the budget grows) fires the
                    // Resume trigger; until then the run is parked here.
                    let reason = self.run.paused.unwrap_or(crate::fsm::PauseReason::Budget);
                    return Ok(Outcome::Paused { reason });
                }
                Phase::Delivered => {
                    self.sync_run_row()?;
                    return Ok(Outcome::Delivered);
                }
                Phase::NeedsUser | Phase::Failed => {
                    let why = format!("run parked at phase {}", self.run.phase.as_str());
                    self.sync_run_row()?;
                    return if self.run.phase == Phase::NeedsUser {
                        Ok(Outcome::NeedsUser { why })
                    } else {
                        Ok(Outcome::Failed { why })
                    };
                }
                p => {
                    return Err(Error::ConfigInvalid(format!(
                        "orchestrator: phase {p:?} is A1/A2 agent territory (Faz 5), not the mini-run driver"
                    )))
                }
            }
        }
    }

    // ---- the planning lifecycle (§6.1 pre-scheduling) ---------------------

    /// Drive the run from `idle` to `Scheduling`: form gate (a REAL structural
    /// check — an incomplete form parks as NeedsUser, never a silent
    /// continuation), A1's draft (ledger-guarded turn), the §9 spec gate, A2's
    /// plan and task-row registration. Deterministic; resume re-enters the
    /// current phase and every turn is ledger-idempotent (§7.4 rule 4).
    pub fn advance_planning(&mut self, pp: &mut dyn PlanningPipeline) -> Result<PlanningOutcome> {
        loop {
            match self.run.phase {
                Phase::Idle => {
                    pp.save_form(self)?;
                    self.fire(Trigger::SpecSubmitted)?;
                }
                Phase::SpecReceived => {
                    // The form check is CODE's, not the model's: an
                    // under-specified form escalates to the user (§6.1
                    // FormInvalidExhausted) instead of being drafted anyway.
                    let bytes = self.read_saved_form()?;
                    let problems = crate::validate::check_form(&bytes);
                    if problems.is_empty() {
                        self.fire(Trigger::FormValidated)?;
                    } else {
                        self.fire(Trigger::FormInvalidExhausted)?;
                        return Ok(PlanningOutcome::NeedsUser {
                            why: problems.join("; "),
                        });
                    }
                }
                Phase::Clarifying => {
                    // Non-interactive fast path: the form arrived complete by
                    // construction (§5.4's Q&A loop replaces this arm when
                    // the interview flow lands — the phase is kept in the
                    // stream either way so the record never skips a beat).
                    self.fire(Trigger::ClarificationsResolved)?;
                }
                Phase::Researching => {
                    if let Err(e) = pp.draft_spec(self) {
                        if self.note_budget_refusal(&e) {
                            return self.budget_pause_outcome_planning();
                        }
                        return Err(e);
                    }
                    self.fire(Trigger::ResearchComplete)?;
                }
                Phase::Advising => {
                    if !pp.lock_decisions(self)? {
                        // Parked IN Advising: no FSM transition lies about
                        // progress; the row stays truthful for resume.
                        return Ok(PlanningOutcome::NeedsUser {
                            why: "spec decisions not locked (under-specified draft)".into(),
                        });
                    }
                    self.fire(Trigger::DecisionsLocked)?;
                }
                Phase::SpecValidated => match self.pass_gate(Gate::Spec)? {
                    GateStep::Passed => {
                        let plan = match pp.plan_tasks(self) {
                            Ok(p) => p,
                            Err(e) => {
                                if self.note_budget_refusal(&e) {
                                    return self.budget_pause_outcome_planning();
                                }
                                return Err(e);
                            }
                        };
                        self.fire(Trigger::GateSpecApproved)?;
                        self.register_plan_rows(&plan)?;
                        self.absorb_plan(&plan)?;
                        self.fire(Trigger::TaskGraphValidated)?;
                    }
                    GateStep::Waiting => {
                        return Ok(PlanningOutcome::Waiting { gate: Gate::Spec });
                    }
                    GateStep::Vetoed => {
                        return Ok(PlanningOutcome::NeedsUser {
                            why: "spec gate vetoed".into(),
                        });
                    }
                },
                Phase::TasksCreated => {
                    // Crash after row registration, before the transition:
                    // rows are durable, just finish the handshake.
                    self.fire(Trigger::TaskGraphValidated)?;
                }
                Phase::Scheduling => return Ok(PlanningOutcome::Planned),
                Phase::Delivered | Phase::Failed | Phase::NeedsUser | Phase::Paused => {
                    return Err(Error::ConfigInvalid(format!(
                        "advance_planning on a {} run — this lifecycle only drives idle..scheduling",
                        self.run.phase.as_str()
                    )));
                }
                p => {
                    return Err(Error::ConfigInvalid(format!(
                        "orchestrator: phase {p:?} is not a planning phase"
                    )))
                }
            }
        }
    }

    /// True when an error from a planning hat is the call-boundary money
    /// wall (as opposed to a real planning failure, which must surface).
    fn note_budget_refusal(&self, e: &Error) -> bool {
        matches!(e, Error::ModelBudgetDenied(_))
    }

    fn budget_pause_outcome_planning(&mut self) -> Result<PlanningOutcome> {
        self.fire(Trigger::BudgetHardLimit)?;
        Ok(PlanningOutcome::Paused {
            reason: crate::fsm::PauseReason::Budget,
        })
    }

    fn read_saved_form(&self) -> Result<Vec<u8>> {
        let p = self
            .cfg
            .root
            .join("runs")
            .join(&self.cfg.spec_id)
            .join("raw_input.md");
        std::fs::read(&p).map_err(|e| {
            Error::ConfigInvalid(format!(
                "run form missing at {} — save_form must run first: {e}",
                p.display()
            ))
        })
    }

    /// Completed task ids per the persisted rows — the immutability set a
    /// replan must respect (rule 4 is the ledger's; this is the plan's).
    pub fn done_task_ids(&self) -> Vec<String> {
        self.sched
            .statuses()
            .iter()
            .filter(|(_, (st, _))| *st == Status::Done)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Absorb a REVISED plan (§5.6 m5). Completed rows are untouched;
    /// failed/blocked rows for tasks the CEO kept or re-cut return to
    /// `pending` with retry_count climbing (the §8.1 attempt record);
    /// rows for tasks the revised plan dropped are deleted (skipped work
    /// leaves no zombie row — the plan document is the task-set truth).
    pub fn absorb_revised_plan(&mut self, plan: &TaskPlan) -> Result<()> {
        let conn = self.store.conn();
        let existing: Vec<(String, String, i64)> = {
            let mut stmt =
                conn.prepare("SELECT task_id, status, retry_count FROM tasks WHERE spec_id = ?1")?;
            let it = stmt.query_map(rusqlite::params![self.cfg.spec_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?;
            it.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let plan_ids: Vec<&str> = plan.tasks.iter().map(|t| t.id.as_str()).collect();
        for (id, status, _retries) in &existing {
            if status == "done" {
                continue; // immutable, rule 4
            }
            if !plan_ids.contains(&id.as_str()) {
                conn.execute(
                    "DELETE FROM tasks WHERE spec_id = ?1 AND task_id = ?2",
                    rusqlite::params![self.cfg.spec_id, id],
                )?;
            }
        }
        for t in &plan.tasks {
            match existing
                .iter()
                .find(|(id, st, _)| id == &t.id && st == "done")
            {
                Some(_) => continue, // keep the done row exactly as persisted
                None => {
                    let was = existing.iter().find(|(id, _, _)| id == &t.id);
                    let retry = was.map(|(_, _, r)| r + 1).unwrap_or(0);
                    conn.execute(
                        "INSERT OR REPLACE INTO tasks(spec_id, task_id, status, agent, retry_count, escalation_level)
                         VALUES (?1, ?2, 'pending', ?3, ?4, 0)",
                        rusqlite::params![self.cfg.spec_id, t.id, t.assigned_agent, retry],
                    )?;
                }
            }
        }
        self.absorb_plan(plan)
    }

    /// Swap the driver's in-memory DAG to the just-planned task set: `by_id`
    /// and a fresh scheduler synced to the registered rows. `create_idle`
    /// builds with an EMPTY plan (nothing exists pre-gate); without this the
    /// Scheduling arm would see a graph that disagrees with the DB (§7.1).
    fn absorb_plan(&mut self, plan: &TaskPlan) -> Result<()> {
        let mut sched = Scheduler::from_plan(plan, self.cfg.max_concurrency)
            .map_err(|e| Error::Scheduler(e.to_string()))?;
        for (id, st) in task_rows(self.store.conn(), &self.cfg.spec_id)? {
            sched.set_status(&id, st);
        }
        self.sched = sched;
        self.by_id = plan
            .tasks
            .iter()
            .map(|t| (t.id.clone(), t.clone()))
            .collect();
        Ok(())
    }

    /// Task-row registration AFTER the spec gate passed (§7.1: the tasks
    /// table only ever grows through this door). Idempotent: a crash between
    /// the `GateSpecApproved` transition and a later re-entry re-runs it.
    fn register_plan_rows(&mut self, plan: &TaskPlan) -> Result<()> {
        let conn = self.store.conn();
        for t in &plan.tasks {
            conn.execute(
                "INSERT OR REPLACE INTO tasks(spec_id, task_id, status, agent, retry_count, escalation_level)
                 VALUES (?1, ?2, 'pending', ?3, 0, 0)",
                params![self.cfg.spec_id, t.id, t.assigned_agent],
            )?;
        }
        Ok(())
    }

    /// §8.2: the locked spec's budget becomes the run's wall (the spec is the
    /// budget authority once validated — the config default only pre-dates it).
    pub fn set_run_budget(&mut self, usd: f64) -> Result<()> {
        self.cfg.budget_usd = usd;
        self.store.conn().execute(
            "UPDATE runs SET budget_usd = ?2 WHERE spec_id = ?1",
            params![self.cfg.spec_id, usd],
        )?;
        Ok(())
    }

    /// §7.4 for a PLANNING turn (kind `plan_turn`): the same exactly-once
    /// discipline as a task — deterministic op identity, persisted result
    /// envelope, charge-guarded spend, kill windows covered by the shared
    /// `Stage` arms. A finished op returns its stored document and NEVER
    /// re-calls the work closure (§7.4 rule 4 — a resumed draft does not
    /// re-invoice the model for a result already on disk).
    pub fn run_op(
        &mut self,
        hint: &str,
        payload: &Value,
        work: &mut dyn FnMut() -> Result<(Value, f64)>,
    ) -> Result<Value> {
        let op = Ledger::new(self.store.conn()).start(
            ledger::kind::PLAN_TURN,
            &self.cfg.spec_id,
            Some(hint),
            payload,
            &stamp(&self.cfg),
        )?;
        // Rule 4: finished ⇒ the stored document IS the answer.
        if Ledger::new(self.store.conn()).is_finished(&op)? {
            let stored = Ledger::new(self.store.conn())
                .get(&op)?
                .ok_or_else(|| Error::Ledger(format!("op {op} vanished mid-check")))?;
            let (doc, _) = self.read_result(&stored)?;
            return Ok(doc);
        }
        self.hit(Stage::AfterStarted)?;
        let (doc, cost) = work()?;
        self.persist_result(
            &op,
            &Executed {
                summary: doc.clone(),
                ok: true,
                cost_usd: cost,
                budget_denied: false,
            },
        )?;
        self.hit(Stage::AfterWork)?;
        self.commit_plan_op(&op, cost)?;
        self.hit(Stage::AfterFinish)?;
        Ok(doc)
    }

    /// The `plan_turn` close: charge-guarded spend + ledger finish — the
    /// task-free sibling of [`commit_task`] (no task rows, no task event).
    fn commit_plan_op(&mut self, op_id: &str, charge: f64) -> Result<()> {
        let charge = charge.max(0.0);
        let mut was_charged = false;
        let mut fired_soft = false;
        {
            let (spec, ts) = (self.cfg.spec_id.clone(), stamp(&self.cfg));
            let conn = self.store.conn();
            conn.execute_batch("BEGIN IMMEDIATE")?;
            let body = (|| -> Result<()> {
                let charged = Ledger::new(conn).charge_once(op_id, &ts)?;
                was_charged = charged;
                if charged {
                    let crosses_soft = !self.budget_warned
                        && self.cfg.budget_usd > 0.0
                        && self.cfg.budget_soft_pct > 0.0
                        && self.spent_usd + charge
                            >= self.cfg.budget_usd * self.cfg.budget_soft_pct / 100.0;
                    conn.execute(
                        "UPDATE runs SET updated_at = ?2, spent_usd = spent_usd + ?3,
                         budget_warned = MAX(budget_warned, ?4) WHERE spec_id = ?1",
                        params![spec, ts, charge, if crosses_soft { 1 } else { 0 }],
                    )?;
                    fired_soft = crosses_soft;
                } else {
                    conn.execute(
                        "UPDATE runs SET updated_at = ?2 WHERE spec_id = ?1",
                        params![spec, ts],
                    )?;
                }
                Ok(())
            })();
            match body {
                Ok(()) => {
                    conn.execute_batch("COMMIT")?;
                    if was_charged {
                        self.spent_usd += charge;
                    }
                    if fired_soft {
                        self.budget_warned = true;
                    }
                }
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    return Err(e);
                }
            }
        }
        let ts = stamp(&self.cfg);
        Ledger::new(self.store.conn()).finish(op_id, None, &ts)?;
        if fired_soft {
            self.events.append_with(
                Ev::BudgetWarning,
                "op-budget",
                None,
                &[
                    ("spent_usd", json!(self.spent_usd)),
                    ("budget_usd", json!(self.cfg.budget_usd)),
                    ("soft_pct", json!(self.cfg.budget_soft_pct)),
                ],
            )?;
        }
        Ok(())
    }

    /// One task through the §7.4 pipeline. Cloning the item keeps the
    /// `&mut self` writes borrow-clean.
    fn run_one_task(&mut self, id: &str, exec: &dyn TaskExecutor) -> Result<()> {
        let task = self.by_id[id].clone();
        let payload = scheduler::task_payload(&task);
        // §7.4 rule 4 — "kanıt ledger.finished kaydıdır": a finished op is
        // never re-executed, no matter what the task row or scheduler thinks.
        // This guards the one path the state machine cannot see: a misused
        // `create` on a spec that already has ledger history.
        let op = Ledger::op_id(lk::AGENT_CALL, &self.cfg.spec_id, Some(id), &payload);
        if Ledger::new(self.store.conn()).is_finished(&op)? {
            tracing::warn!(task = id, op = %op, "ledger says finished — skipping (resume rule 4)");
            // The ledger row is the PROOF (§7.4 rule 4). Restore the
            // scheduler/DB from that proof — otherwise a reset task row
            // (misused `create`) would spin the loop on work that is done.
            self.sched.set_status(id, Status::Done);
            self.write_task_row(id, Status::Done, 0)?;
            return Ok(());
        }
        let op = Ledger::new(self.store.conn()).start(
            lk::AGENT_CALL,
            &self.cfg.spec_id,
            Some(id),
            &payload,
            &stamp(&self.cfg),
        )?;
        self.hit(Stage::AfterStarted)?;
        let outcome = exec.execute(&task, &op)?;
        if outcome.budget_denied {
            // §8.2/B3: money refusal is not a task defect. The op is closed
            // as failed WITHOUT a result (resume re-derives the same op_id
            // and retries once the wall rises — rule 4 guards only
            // `finished`); the task row goes back to pending, no blocked
            // cascade, and the run pauses with completed work kept.
            let ts = stamp(&self.cfg);
            Ledger::new(self.store.conn()).fail(&op, &ts)?;
            self.write_task_row(id, Status::Pending, 0)?;
            self.sched.set_status(id, Status::Pending);
            tracing::warn!(task = %id, "call-boundary budget wall hit — pausing run");
            self.budget_paused = true;
            return Ok(());
        }
        // The result file is persisted BEFORE any state.db write so the §7.4
        // "only the writing was interrupted" recovery branch exists at all —
        // but ONLY for a successful outcome. Writing a result_ref on a failed
        // op would let a mid-fail crash recover as `FinalizeResult` (success),
        // silently flipping a failure. Failed ops carry no result to finalize.
        if outcome.ok {
            self.persist_result(&op, &outcome)?;
            self.hit(Stage::AfterWork)?;
        }
        // The real spend of the execution is booked even when the task failed:
        // the LLM/tool already ran (ok=false ≠ refund).
        self.commit_task(
            &op,
            Some(id),
            &outcome.summary,
            outcome.ok,
            outcome.cost_usd,
        )?;
        Ok(())
    }

    fn vetoed(&self, gate: Gate) -> Outcome {
        Outcome::NeedsUser {
            why: format!("gate {} vetoed", gate.as_str()),
        }
    }

    fn fire(&mut self, t: Trigger) -> Result<()> {
        let step = fsm::transition(self.run, t, self.cfg.crisis_max).map_err(fsm_err)?;
        if let Some(ev) = step.event {
            self.events.append(ev, "op-fsm", None, Default::default())?;
        }
        self.run = step.state;
        self.sync_run_row()
    }

    // ---- gates (§9) -------------------------------------------------------

    /// One gate decision cycle (§9): read the durable `approvals` state (a
    /// `pending` row is the persisted REQUEST — its `ts` anchors the timeout
    /// clock, so the §9 window survives run→resume process boundaries),
    /// apply the policy, record the audit row + events. Never sleeps: the
    /// timeout is evaluated against the injected clock.
    fn pass_gate(&mut self, gate: Gate) -> Result<GateStep> {
        let existing = gates::Approvals::new(self.store.conn()).lookup(&self.cfg.spec_id, gate)?;
        let requested_at = existing
            .as_ref()
            .filter(|r| r.decision == gates::decision::PENDING)
            .and_then(|r| crate::clock::epoch_from_utc_iso(&r.ts))
            .unwrap_or(self.cfg.now_epoch);
        let out = gates::decide(gates::GateRequest {
            gate,
            autonomy_level: self.cfg.autonomy_level,
            batch_yes: self.cfg.batch_yes,
            requested_at_epoch: requested_at,
            now_epoch: self.cfg.now_epoch,
            timeout_seconds: self.cfg.gate_timeout_seconds,
            existing: existing.as_ref(),
        });
        match &out {
            GateOutcome::StillWaiting => {
                // Record the request EXACTLY ONCE. Re-stamping `pending` on
                // every poll would reset the §9 timeout forever — the gate
                // could never auto-approve, silently violating the Q4 rule.
                let fresh = existing.is_none();
                if fresh {
                    gates::Approvals::new(self.store.conn()).record(
                        &self.cfg.spec_id,
                        gate,
                        "gate-request",
                        gates::decision::PENDING,
                        None,
                        &stamp(&self.cfg),
                    )?;
                }
                self.events.append_with(
                    Ev::HumanApprovalRequested,
                    "op-gate",
                    None,
                    &[("gate", json!(gate.as_str()))],
                )?;
                Ok(GateStep::Waiting)
            }
            GateOutcome::Denied { by } => {
                self.events.append_with(
                    Ev::HumanApprovalDenied,
                    "op-gate",
                    None,
                    &[("gate", json!(gate.as_str())), ("by", json!(by))],
                )?;
                self.convert_gate_outcome(Trigger::GateDenied)?;
                Ok(GateStep::Vetoed)
            }
            GateOutcome::Park => {
                self.events.append_with(
                    Ev::GateTimeout,
                    "op-gate",
                    None,
                    &[("gate", json!(gate.as_str())), ("reason", json!("no_auto"))],
                )?;
                self.convert_gate_outcome(Trigger::GateParked)?;
                Ok(GateStep::Vetoed)
            }
            GateOutcome::Granted(reason) => {
                let reason_s = gates::reason_str(*reason);
                if matches!(reason, GrantedReason::TimeoutAuto) {
                    self.events.append_with(
                        Ev::GateTimeout,
                        "op-gate",
                        None,
                        &[("gate", json!(gate.as_str()))],
                    )?;
                }
                gates::Approvals::new(self.store.conn()).record(
                    &self.cfg.spec_id,
                    gate,
                    &decided_by(*reason, &existing),
                    if matches!(reason, GrantedReason::TimeoutAuto) {
                        "timeout_auto"
                    } else {
                        "approved"
                    },
                    Some(reason_s),
                    &stamp(&self.cfg),
                )?;
                self.events.append_with(
                    Ev::HumanApprovalGranted,
                    "op-gate",
                    None,
                    &[("gate", json!(gate.as_str())), ("reason", json!(reason_s))],
                )?;
                Ok(GateStep::Passed)
            }
        }
    }

    /// Denial/park must move the FSM itself — the phase row stays the single
    /// truth (§7.1), never a side-channel.
    fn convert_gate_outcome(&mut self, t: Trigger) -> Result<()> {
        let step = fsm::transition(self.run, t, self.cfg.crisis_max).map_err(fsm_err)?;
        if let Some(ev) = step.event {
            self.events
                .append(ev, "op-gate", None, Default::default())?;
        }
        self.run = step.state;
        self.sync_run_row()
    }

    // ---- the §7.4 write order --------------------------------------------

    /// Steps (2)–(4) for one task outcome. Scheduler mirrors the persisted
    /// result LAST, after every write — an aborted commit can never make it
    /// believe a task finished.
    fn commit_task(
        &mut self,
        op_id: &str,
        task_id: Option<&str>,
        summary: &Value,
        ok: bool,
        charge_usd: f64,
    ) -> Result<()> {
        let status = if ok { Status::Done } else { Status::Failed };
        let charge = charge_usd.max(0.0);
        // A failure's `blocked` cascade (§6.2) is computed on a CLONE so the
        // newly-blocked ids can be written inside the SAME transaction as the
        // failed row — §7.1's single truth must never show a dead dependency
        // with a still-`pending` child after a kill. self.sched is mirrored
        // LAST (below), so an aborted commit can't make it believe a task ran.
        let cascade: Vec<String> = match (ok, task_id) {
            (false, Some(t)) => {
                let mut mirror = self.sched.clone();
                mirror.mark_unreachable(t, Status::Failed)
            }
            _ => Vec::new(),
        };
        // (2) state.db transaction — one BEGIN IMMEDIATE commit per §7.4. The
        // cost accrual rides in the SAME tx so `runs.spent_usd` can never
        // disagree with the task statuses it is derived from (§8.2/§7.1).
        // §8.2 charge discipline (migration 0003): `charged_at` flips
        // NULL→stamp and the RELATIVE increment share this one transaction —
        // a kill between this commit and the ledger close previously
        // double-charged on resume (the FinalizeResult branch re-books the
        // persisted cost on top of a row that already carried it; invisible
        // while every executor billed 0.0). The soft-latch write is sticky
        // (MAX) — a non-crossing commit must never erase a prior warning.
        let mut was_charged = false;
        let mut fired_soft = false;
        {
            let (spec, ts) = (self.cfg.spec_id.clone(), stamp(&self.cfg));
            let conn = self.store.conn();
            conn.execute_batch("BEGIN IMMEDIATE")?;
            let body = (|| -> Result<()> {
                if let Some(t) = task_id {
                    conn.execute(
                        "UPDATE tasks SET status = ?3 WHERE spec_id = ?1 AND task_id = ?2",
                        params![spec, t, status.as_str()],
                    )?;
                }
                for b in &cascade {
                    conn.execute(
                        "UPDATE tasks SET status = 'blocked' WHERE spec_id = ?1 AND task_id = ?2",
                        params![spec, b],
                    )?;
                }
                let charged = Ledger::new(conn).charge_once(op_id, &ts)?;
                was_charged = charged;
                if charged {
                    let crosses_soft = !self.budget_warned
                        && self.cfg.budget_usd > 0.0
                        && self.cfg.budget_soft_pct > 0.0
                        && self.spent_usd + charge
                            >= self.cfg.budget_usd * self.cfg.budget_soft_pct / 100.0;
                    conn.execute(
                        "UPDATE runs SET updated_at = ?2, spent_usd = spent_usd + ?3,
                         budget_warned = MAX(budget_warned, ?4) WHERE spec_id = ?1",
                        params![spec, ts, charge, if crosses_soft { 1 } else { 0 }],
                    )?;
                    fired_soft = crosses_soft;
                } else {
                    // A previous process already booked this op's cost
                    // (FinalizeResult recovery across the kill window): keep
                    // the row's spend, refresh only the timestamp.
                    conn.execute(
                        "UPDATE runs SET updated_at = ?2 WHERE spec_id = ?1",
                        params![spec, ts],
                    )?;
                }
                Ok(())
            })();
            match body {
                Ok(()) => {
                    conn.execute_batch("COMMIT")?;
                    // Mirror the committed spend in memory only AFTER commit —
                    // a rolled-back tx must leave self.spent_usd truthful too.
                    if was_charged {
                        self.spent_usd += charge;
                    }
                    if fired_soft {
                        self.budget_warned = true;
                    }
                }
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    return Err(e);
                }
            }
        }
        self.hit(Stage::AfterDoneTx)?;
        // (3) events.jsonl append (fsynced inside EventLog).
        self.events.append_with(
            if ok {
                Ev::TaskCompleted
            } else {
                Ev::TaskFailed
            },
            op_id,
            task_id,
            &[("summary", summary.clone())],
        )?;
        self.hit(Stage::AfterDoneEvent)?;
        // (4) ledger close.
        let result_ref = task_result_path(&self.cfg.root, &self.cfg.spec_id, op_id);
        let ts = stamp(&self.cfg);
        let led = Ledger::new(self.store.conn());
        if ok {
            led.finish(op_id, Some(&result_ref), &ts)?;
        } else {
            led.fail(op_id, &ts)?;
        }
        self.hit(Stage::AfterFinish)?;
        if fired_soft {
            // §8.2 soft wall: one-time, latch already committed; emitted after
            // the op's four writes so it never interleaves the §7.4 window.
            self.events.append_with(
                Ev::BudgetWarning,
                "op-budget",
                None,
                &[
                    ("spent_usd", json!(self.spent_usd)),
                    ("budget_usd", json!(self.cfg.budget_usd)),
                    ("soft_pct", json!(self.cfg.budget_soft_pct)),
                ],
            )?;
        }
        if let Some(t) = task_id {
            // Mirror the in-memory scheduler LAST (DB was already written
            // atomically above). Idempotent re-run of the cascade; the return
            // (newly-blocked ids) is discarded because persistence happened.
            if ok {
                self.sched.set_status(t, Status::Done);
            } else {
                let _ = self.sched.mark_unreachable(t, Status::Failed);
            }
        }
        Ok(())
    }

    fn write_task_row(&self, task_id: &str, s: Status, retry: u32) -> Result<()> {
        self.store.conn().execute(
            "UPDATE tasks SET status = ?3, retry_count = ?4 WHERE spec_id = ?1 AND task_id = ?2",
            params![self.cfg.spec_id, task_id, s.as_str(), retry],
        )?;
        Ok(())
    }

    fn sync_run_row(&self) -> Result<()> {
        self.store.conn().execute(
            "UPDATE runs SET status = ?2, crisis_count = ?3, updated_at = ?4,
             spent_usd = ?5, budget_warned = ?6, paused_from = ?7, paused_reason = ?8
             WHERE spec_id = ?1",
            params![
                self.cfg.spec_id,
                self.run.phase.as_str(),
                self.run.crisis_count as i64,
                stamp(&self.cfg),
                self.spent_usd,
                self.budget_warned as i64,
                self.run.paused_from.map(|p| p.as_str()),
                self.run.paused.map(|r| match r {
                    crate::fsm::PauseReason::Budget => "budget",
                    crate::fsm::PauseReason::Checkpoint => "checkpoint",
                }),
            ],
        )?;
        Ok(())
    }

    /// §8.2/B3: after the user raises `budget_usd` (a fresh RunConfig carries
    /// the new wall), un-park a `Paused(budget)` run back to its recorded
    /// phase. Returns false if the run was not paused (nothing to do).
    pub fn unpause(&mut self) -> Result<bool> {
        if self.run.phase != Phase::Paused {
            return Ok(false);
        }
        self.fire(Trigger::Resume)?;
        Ok(true)
    }

    /// Live spend mirror (tests + the report surface read truth, not the file).
    pub fn spent_usd(&self) -> f64 {
        self.spent_usd
    }

    /// Read-only view of the run configuration the driver was built with
    /// (paths + knobs for the seams that write run documents).
    pub fn config(&self) -> &RunConfig {
        &self.cfg
    }

    // ---- results & digest -------------------------------------------------

    fn persist_result(&self, op_id: &str, outcome: &Executed) -> Result<String> {
        let path = task_result_path(&self.cfg.root, &self.cfg.spec_id, op_id);
        std::fs::create_dir_all(Path::new(&path).parent().unwrap())?;
        // Envelope, not bare summary: §7.4's FinalizeResult recovery re-commits
        // the done-transaction WITHOUT re-running the executor, so the cost
        // actually spent must be re-readable from the persisted result — else
        // `spent_usd` would silently under-count after a crash at AfterWork.
        let envelope = json!({ "summary": outcome.summary, "cost_usd": outcome.cost_usd });
        std::fs::write(&path, hash::canonical_json(&envelope))?;
        Ledger::new(self.store.conn()).set_result(op_id, &path)?;
        Ok(path)
    }

    /// Read back `(summary, cost_usd)` from a persisted result envelope.
    /// Pre-envelope files (no wrapper) are tolerated: summary = whole doc,
    /// cost 0.
    fn read_result(&self, op: &ledger::Op) -> Result<(Value, f64)> {
        let p = op
            .result_ref
            .as_deref()
            .ok_or_else(|| Error::Ledger(format!("op {} has no result_ref", op.op_id)))?;
        let raw = std::fs::read_to_string(p)?;
        let doc: Value = serde_json::from_str(&raw)?;
        let cost = doc.get("cost_usd").and_then(Value::as_f64).unwrap_or(0.0);
        let summary = doc.get("summary").cloned().unwrap_or(doc);
        Ok((summary, cost))
    }

    /// §21.2 golden comparator: SHA-256 over the NORMALIZED final state —
    /// phase, crisis counter, task statuses, ledger outcomes and the SET of
    /// logical events. seq/ts/run_id/paths are excluded by design: the claim
    /// is "the same outcome was reached", and duplicate events re-emitted by
    /// recovery inside the B4 window collapse under set normalization —
    /// which is exactly what idempotent resume guarantees.
    pub fn final_digest(&self) -> Result<String> {
        let conn = self.store.conn();
        let tasks = query_strings(
            conn,
            &format!(
                "SELECT task_id || '|' || status || '|' || retry_count FROM tasks WHERE spec_id = '{}' ORDER BY task_id",
                self.cfg.spec_id
            ),
        )?;
        let ops = query_strings(
            conn,
            &format!(
                "SELECT DISTINCT kind || '|' || COALESCE(task_id,'') || '|' || state || '|' || payload_hash FROM ledger WHERE spec_id = '{}' ORDER BY 1",
                self.cfg.spec_id
            ),
        )?;
        let mut events: Vec<String> = Vec::new();
        for ev in self.events.replay()? {
            events.push(format!(
                "{}|{}|{}",
                ev.event.as_str(),
                ev.task_id.unwrap_or_default(),
                hash::canonical_json(&serde_json::to_value(&ev.data)?)
            ));
        }
        events.sort();
        events.dedup();
        let doc = json!({
            "phase": self.run.phase.as_str(),
            "crisis": self.run.crisis_count,
            "tasks": tasks,
            "ops": ops,
            "events": events,
        });
        Ok(hash::sha256_hex(hash::canonical_json(&doc).as_bytes()))
    }

    pub fn phase(&self) -> Phase {
        self.run.phase
    }
}

enum GateStep {
    Passed,
    Waiting,
    Vetoed,
}

fn decided_by(reason: GrantedReason, existing: &Option<ApprovalRow>) -> String {
    match reason {
        GrantedReason::Human => existing
            .as_ref()
            .map(|r| r.decided_by.clone())
            .unwrap_or_else(|| "user".into()),
        GrantedReason::BatchYes => "batch-yes".into(),
        GrantedReason::LevelAuto => "autonomy-policy".into(),
        GrantedReason::TimeoutAuto => "gate-timeout".into(),
    }
}

fn fsm_err(e: FsmError) -> Error {
    Error::Fsm(e.to_string())
}

fn stamp(cfg: &RunConfig) -> String {
    crate::clock::fmt_epoch_seconds(cfg.now_epoch)
}

fn run_dir(root: &Path, spec_id: &str) -> PathBuf {
    root.join("runs").join(spec_id)
}

/// Artifact paths are forward-slash strings even on Windows — hashes and
/// event payloads must be byte-stable across the CI matrix (§21.3).
fn task_result_path(root: &Path, spec_id: &str, op_id: &str) -> String {
    run_dir(root, spec_id)
        .join("artifacts")
        .join(format!("{op_id}.json"))
        .to_string_lossy()
        .replace('\\', "/")
}

fn phase_of_str(s: &str) -> Option<Phase> {
    [
        Phase::Idle,
        Phase::SpecReceived,
        Phase::Clarifying,
        Phase::Researching,
        Phase::Advising,
        Phase::SpecValidated,
        Phase::TasksCreated,
        Phase::Scheduling,
        Phase::Finalizing,
        Phase::FinalReview,
        Phase::Crisis,
        Phase::Delivered,
        Phase::NeedsUser,
        Phase::Failed,
        Phase::Paused,
    ]
    .into_iter()
    .find(|p| p.as_str() == s)
}

fn task_rows(conn: &rusqlite::Connection, spec_id: &str) -> Result<Vec<(String, Status)>> {
    let mut stmt = conn.prepare("SELECT task_id, status FROM tasks WHERE spec_id = ?1")?;
    let rows = stmt.query_map(params![spec_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, st) = row?;
        let status = Status::parse(&st)
            .ok_or_else(|| Error::Ledger(format!("tasks.status {st:?} unknown")))?;
        out.push((id, status));
    }
    Ok(out)
}

fn query_strings(conn: &rusqlite::Connection, sql: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

// ---------------------------------------------------------------- tests --

#[cfg(test)]
mod planning_tests {
    use super::*;
    use crate::state::StateStore;
    use crate::validate;
    use std::cell::RefCell;

    fn cfg(root: &Path, autonomy: u8, yes: bool) -> RunConfig {
        RunConfig {
            root: root.to_path_buf(),
            spec_id: "spec-20260912-plan".into(),
            run_id: "run-plan".into(),
            autonomy_level: autonomy,
            batch_yes: yes,
            max_concurrency: 1,
            crisis_max: 2,
            gate_timeout_seconds: 1200,
            budget_usd: 5.0,
            budget_soft_pct: 80.0,
            budget_hard_pct: 100.0,
            now_epoch: 1_789_207_200,
        }
    }

    const FORM: &[u8] = b"# Spec Form: tiny\n\n## Summary\nadd two numbers\n\n## Goal & Users\n- goal: demo\n\n## Scope\n- must_have:\n  - add()\n\n## Constraints\n- rust only\n\n## Budget\n- run_budget_usd: 1.5\n\n## Delivery Shape\ncrate\n\n## Success Criteria\ncargo test passes\n";

    fn one_task_plan() -> TaskPlan {
        let doc = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/expected/tasks_valid.json"),
        )
        .unwrap();
        let mut plan: TaskPlan = serde_json::from_str(&doc).unwrap();
        plan.spec_id = "spec-20260912-plan".into();
        plan
    }

    /// Deterministic fake of the management hats: saves the form, produces a
    /// fixed spec document through the ledger, plans from the fixture.
    struct FakePlanning {
        spec_doc: Value,
        form: Vec<u8>,
        calls: std::cell::RefCell<Vec<String>>,
    }
    impl FakePlanning {
        fn new() -> Self {
            Self {
                spec_doc: serde_json::json!({"spec": "v1"}),
                form: FORM.to_vec(),
                calls: RefCell::new(vec![]),
            }
        }
    }
    impl PlanningPipeline for FakePlanning {
        fn save_form(&mut self, o: &mut Orchestrator) -> Result<()> {
            let dir = o.config().root.join("runs").join(&o.config().spec_id);
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("raw_input.md"), &self.form)?;
            Ok(())
        }
        fn draft_spec(&mut self, o: &mut Orchestrator) -> Result<()> {
            let payload = serde_json::json!({"turn": "a1-draft"});
            let doc = o.run_op("a1-draft", &payload, &mut || {
                // Record INSIDE the work closure: a rule-4 skip (finished op)
                // must not count as a re-invocation.
                self.calls.borrow_mut().push("draft".into());
                Ok((self.spec_doc.clone(), 0.5))
            })?;
            o.set_run_budget(1.5)?;
            std::fs::write(
                o.config()
                    .root
                    .join("runs")
                    .join(&o.config().spec_id)
                    .join("spec.json"),
                doc.to_string(),
            )?;
            Ok(())
        }
        fn lock_decisions(&mut self, _o: &mut Orchestrator) -> Result<bool> {
            Ok(true)
        }
        fn plan_tasks(&mut self, o: &mut Orchestrator) -> Result<TaskPlan> {
            self.calls.borrow_mut().push("plan".into());
            let payload = serde_json::json!({"turn": "a2-plan"});
            let doc = o.run_op("a2-plan", &payload, &mut || {
                Ok((serde_json::to_value(one_task_plan())?, 0.25))
            })?;
            std::fs::write(
                o.config()
                    .root
                    .join("runs")
                    .join(&o.config().spec_id)
                    .join("plan.json"),
                doc.to_string(),
            )?;
            Ok(one_task_plan())
        }
    }

    fn stage(root: &Path) -> String {
        let store = StateStore::open(&root.join(".ribozym").join("state.db")).unwrap();
        store
            .conn()
            .query_row("SELECT status FROM runs", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn planning_lifecycle_walks_every_phase_and_lands_in_scheduling() {
        let dir = tempfile::tempdir().unwrap();
        let c = cfg(dir.path(), 3, true);
        let mut o = Orchestrator::create_idle(&c).unwrap();
        let mut pp = FakePlanning::new();
        assert_eq!(
            o.advance_planning(&mut pp).unwrap(),
            PlanningOutcome::Planned
        );
        // Every §6.1 planning phase left its event in the stream, in order.
        let events = std::fs::read_to_string(
            dir.path()
                .join("runs")
                .join("spec-20260912-plan")
                .join("events.jsonl"),
        )
        .unwrap();
        for ev in ["\"SpecReceived\"", "\"SpecValidated\"", "\"TasksCreated\""] {
            assert!(events.contains(ev), "missing {ev} in {events}");
        }
        assert_eq!(stage(dir.path()), "scheduling");
        // Task rows registered exactly once.
        let store = StateStore::open(&dir.path().join(".ribozym").join("state.db")).unwrap();
        let n: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2, "the fixture plan has two tasks");
        // Both planning turns went through the ledger, charged, closed.
        let (turns, charged): (i64, i64) = store
            .conn()
            .query_row(
                "SELECT COUNT(*), SUM(charged_at IS NOT NULL) FROM ledger WHERE kind='plan_turn'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((turns, charged), (2, 2));
        assert!(
            (o.spent_usd() - 0.75).abs() < 1e-9,
            "plan turns bill too, spent {}",
            o.spent_usd()
        );
        // The spec's budget replaced the config default at the run row.
        let b: f64 = store
            .conn()
            .query_row("SELECT budget_usd FROM runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(b, 1.5);
    }

    #[test]
    fn an_incomplete_form_parks_as_needs_user_without_guessing() {
        let dir = tempfile::tempdir().unwrap();
        let c = cfg(dir.path(), 3, true);
        let mut o = Orchestrator::create_idle(&c).unwrap();
        let mut pp = FakePlanning::new();
        pp.form = b"# Spec Form: x\n\n## Summary\n\n".to_vec();
        let out = o.advance_planning(&mut pp).unwrap();
        match &out {
            PlanningOutcome::NeedsUser { why } => {
                assert!(why.contains("must_have"), "{why}");
                assert!(why.contains("## Scope"), "{why}");
            }
            other => panic!("expected NeedsUser, got {other:?}"),
        }
        assert_eq!(stage(dir.path()), "needs_user");
        let events = std::fs::read_to_string(
            dir.path()
                .join("runs")
                .join("spec-20260912-plan")
                .join("events.jsonl"),
        )
        .unwrap();
        assert!(events.contains("\"EscalatedToUser\""));
    }

    #[test]
    fn spec_gate_waits_and_resumes_on_a_durable_approval() {
        let dir = tempfile::tempdir().unwrap();
        let c = cfg(dir.path(), 1, false); // L1: spec gate REQUIRES a human
        let mut o = Orchestrator::create_idle(&c).unwrap();
        let mut pp = FakePlanning::new();
        match o.advance_planning(&mut pp).unwrap() {
            PlanningOutcome::Waiting { gate } => assert_eq!(gate.as_str(), "spec"),
            other => panic!("expected Waiting, got {other:?}"),
        }
        assert_eq!(stage(dir.path()), "spec_validated");
        drop(o);
        // A file vote (`ribozym approve`'s exact write) unblocks the resumed run.
        {
            let store = StateStore::open(&dir.path().join(".ribozym").join("state.db")).unwrap();
            crate::gates::Approvals::new(store.conn())
                .record(
                    "spec-20260912-plan",
                    Gate::Spec,
                    "cli-user",
                    "approved",
                    None,
                    "2026-09-13T00:00:00Z",
                )
                .unwrap();
        }
        let mut r = Orchestrator::resume(&c, &TaskPlan::default()).unwrap();
        assert_eq!(
            r.advance_planning(&mut pp).unwrap(),
            PlanningOutcome::Planned
        );
        assert_eq!(stage(dir.path()), "scheduling");
    }

    #[test]
    fn plan_turn_crash_resume_never_re_invokes_the_model() {
        // Kill in the B4 window AFTER the result envelope persisted: recovery
        // must close the op from the envelope and rule 4 must serve the SAME
        // document on the next pass — the draft work closure runs exactly once.
        let dir = tempfile::tempdir().unwrap();
        let c = cfg(dir.path(), 3, true);
        let mut o = Orchestrator::create_idle(&c).unwrap();
        let mut pp = FakePlanning::new();
        o.arm_crash(Stage::AfterWork);
        let _ = o.advance_planning(&mut pp).unwrap_err(); // crashed in draft
        drop(o);
        let mut r = Orchestrator::resume(&c, &TaskPlan::default()).unwrap();
        assert_eq!(
            r.advance_planning(&mut pp).unwrap(),
            PlanningOutcome::Planned
        );
        assert_eq!(
            *pp.calls.borrow(),
            vec!["draft", "plan"],
            "the crashed draft turn recovered from the envelope, never re-ran"
        );
        // The recovered charge: 0.5 (envelope) + 0.25 (plan) — no double.
        assert!(
            (r.spent_usd() - 0.75).abs() < 1e-9,
            "spent {}",
            r.spent_usd()
        );
    }

    #[test]
    fn resume_without_the_plan_document_refuses_instead_of_guessing() {
        // Rows exist (past TasksCreated) but no plan supplied: a guessed
        // scheduler would disagree with the persisted task set (§7.1).
        let dir = tempfile::tempdir().unwrap();
        let c = cfg(dir.path(), 3, true);
        let mut o = Orchestrator::create_idle(&c).unwrap();
        let mut pp = FakePlanning::new();
        assert_eq!(
            o.advance_planning(&mut pp).unwrap(),
            PlanningOutcome::Planned
        );
        drop(o);
        let err = Orchestrator::resume(&c, &TaskPlan::default()).unwrap_err();
        assert!(err.to_string().contains("plan.json"), "{err}");
    }

    #[test]
    fn planning_wall_hit_pauses_at_the_same_phase_and_resumes() {
        // §8.2/B3 for management turns: the draft hits the call-boundary
        // wall → Paused recording paused_from=researching; the unfinished
        // plan_turn op is re-derived on resume (ReRun branch) and the SAME
        // phase completes once the refusal goes away.
        struct FlakyDraft(FakePlanning, bool);
        impl PlanningPipeline for FlakyDraft {
            fn save_form(&mut self, o: &mut Orchestrator) -> Result<()> {
                self.0.save_form(o)
            }
            fn draft_spec(&mut self, o: &mut Orchestrator) -> Result<()> {
                if self.1 {
                    return Err(Error::ModelBudgetDenied("estimated > remaining".into()));
                }
                self.0.draft_spec(o)
            }
            fn lock_decisions(&mut self, o: &mut Orchestrator) -> Result<bool> {
                self.0.lock_decisions(o)
            }
            fn plan_tasks(&mut self, o: &mut Orchestrator) -> Result<TaskPlan> {
                self.0.plan_tasks(o)
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let c = cfg(dir.path(), 3, true);
        let mut o = Orchestrator::create_idle(&c).unwrap();
        let mut pp = FlakyDraft(FakePlanning::new(), true);
        assert_eq!(
            o.advance_planning(&mut pp).unwrap(),
            PlanningOutcome::Paused {
                reason: crate::fsm::PauseReason::Budget
            }
        );
        assert_eq!(stage(dir.path()), "paused");
        drop(o);
        let mut r = Orchestrator::resume(&c, &TaskPlan::default()).unwrap();
        assert!(r.unpause().unwrap());
        let mut ok = FlakyDraft(FakePlanning::new(), false);
        assert_eq!(
            r.advance_planning(&mut ok).unwrap(),
            PlanningOutcome::Planned
        );
        // The refused draft op retried once and finished; one plan op. Two
        // finished plan_turn rows total — the denial left no phantom result.
        let store = StateStore::open(&dir.path().join(".ribozym").join("state.db")).unwrap();
        let n: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM ledger WHERE kind='plan_turn' AND state='finished'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 2);
    }

    /// The form check itself is the FormValidated decision — contract test
    /// tying the orchestrator arm to the pure checker.
    #[test]
    fn form_check_matches_the_orchestrators_gate() {
        assert!(validate::check_form(FORM).is_empty());
        assert!(!validate::check_form(b"# x\n").is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_round_trips_through_the_db_status_string() {
        for p in [
            Phase::Idle,
            Phase::TasksCreated,
            Phase::Scheduling,
            Phase::Delivered,
            Phase::Paused,
        ] {
            assert_eq!(phase_of_str(p.as_str()), Some(p));
        }
        assert_eq!(phase_of_str("nonsense"), None);
    }

    #[test]
    fn result_path_is_forward_slash_stable_across_platforms() {
        let p = task_result_path(Path::new("work/dir"), "spec-1", "op-abc");
        assert!(!p.contains('\\'), "backslashes would poison hashes: {p}");
        assert!(p.ends_with("runs/spec-1/artifacts/op-abc.json"));
    }

    #[test]
    fn outcomes_are_distinct_no_silent_success() {
        // P3: a parked run must never compare equal to a delivered one.
        assert_ne!(
            Outcome::Delivered,
            Outcome::NeedsUser {
                why: "gate delivery vetoed".into()
            }
        );
        assert_ne!(
            Outcome::Delivered,
            Outcome::Waiting {
                gate: Gate::TaskPlan
            }
        );
    }
}
