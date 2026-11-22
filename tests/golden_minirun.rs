//! Faz 4-b/4-c golden mini-run (§22 exit criterion, §21.2 Uçtan uca katman):
//! 3 fake tasks (one parallel group T2∥T3), kill→resume must converge to
//! the SAME final hash — including every §7.4-B4 discrete-crash window
//! ("4 yazım adımının her biri sonrası") plus the mid-work window. No LLM,
//! no wall clock, no sleeps (§21.1).

use std::cell::RefCell;
use std::path::Path;

use ribozym::gates::{self, Gate};
use ribozym::models::task::{Acceptance, TaskItem, TaskPlan};
use ribozym::orchestrator::{Executed, Orchestrator, Outcome, RunConfig, Stage, TaskExecutor};
use ribozym::state::StateStore;
use serde_json::{json, Value};

// ---------------------------------------------------------------- harness --

/// Deterministic fake agent: same task ⇒ same summary bytes, so a re-run
/// after a crash is bit-for-bit replaceable. `calls` PROVES §7.4 rule 4 —
/// completed work (and finalized results) are never re-executed.
#[derive(Default)]
struct Fake {
    calls: RefCell<Vec<String>>,
}
impl Fake {
    fn new() -> Self {
        Self::default()
    }
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}
impl TaskExecutor for Fake {
    fn execute(&self, task: &TaskItem, _op_id: &str) -> ribozym::Result<Executed> {
        self.calls.borrow_mut().push(task.id.clone());
        Ok(Executed {
            summary: json!({"task": task.id, "agent": task.assigned_agent, "shipped": true}),
            ok: true,
            cost_usd: 0.0,
            budget_denied: false,
        })
    }
}

fn task(id: &str, deps: &[&str], path: &str) -> TaskItem {
    TaskItem {
        id: id.into(),
        spec_requirement_ids: vec!["R1".into()],
        title: format!("golden task {id}"),
        description: String::new(),
        department: None,
        assigned_agent: "A5".into(),
        dependencies: deps.iter().map(|s| s.to_string()).collect(),
        target: "cloud".into(),
        toolchain: None,
        acceptance: Acceptance {
            verifiable_by: "A4".into(),
            criteria: "summary shipped".into(),
        },
        estimated: None,
        difficulty: None,
        file_ownership: vec![path.into()],
        artifacts_expected: vec![path.into()],
        artifacts_produced: vec![],
        blocked_by: vec![],
        status: "pending".into(),
        retry_count: 0,
        escalation_level: 0,
        notes: String::new(),
        extra: Default::default(),
    }
}

/// T1 → {T2 ∥ T3}: disjoint paths so the two children admit together.
fn golden_plan() -> TaskPlan {
    TaskPlan {
        schema_version: "1.0".into(),
        spec_id: SPEC.into(),
        created_at: "2026-09-12T00:00:00Z".into(),
        created_by: "A2".into(),
        revision: 1,
        tasks: vec![
            task("T1", &[], "src/one.rs"),
            task("T2", &["T1"], "src/two.rs"),
            task("T3", &["T1"], "src/three.rs"),
        ],
        execution_order: vec!["T1".into(), "T2".into(), "T3".into()],
        parallel_groups: vec![vec!["T2".into(), "T3".into()]],
        critical_path: vec!["T1".into(), "T2".into()],
        status_summary: Default::default(),
        extra: Default::default(),
    }
}

const SPEC: &str = "spec-20260912-golden";

/// --yes batch mode at L1 (gates auto-approve; interactive gates get their
/// own tests below).
fn cfg(root: &Path) -> RunConfig {
    RunConfig {
        root: root.to_path_buf(),
        spec_id: SPEC.into(),
        run_id: "run-golden".into(),
        autonomy_level: 1,
        batch_yes: true,
        max_concurrency: 2,
        crisis_max: 2,
        gate_timeout_seconds: 1200,
        budget_usd: 5.0,
        budget_soft_pct: 80.0,
        budget_hard_pct: 100.0,
        now_epoch: 1_789_207_200, // 2026-09-12T10:00:00Z (clock.rs constant)
    }
}

fn read_events(root: &Path) -> Vec<(u64, String, Option<String>)> {
    let raw = std::fs::read_to_string(root.join("runs").join(SPEC).join("events.jsonl"))
        .expect("events.jsonl exists");
    raw.lines()
        .filter_map(|l| {
            let v: Value = serde_json::from_str(l).ok()?;
            Some((
                v["seq"].as_u64()?,
                v["event"].as_str()?.to_string(),
                v["task_id"].as_str().map(|s| s.to_string()),
            ))
        })
        .collect()
}

fn clean_run(root: &Path) -> String {
    let fake = Fake::new();
    let mut o = Orchestrator::create(&cfg(root), &golden_plan()).unwrap();
    let out = o.advance(&fake, true).unwrap();
    assert_eq!(out, Outcome::Delivered);
    assert_eq!(
        fake.calls(),
        vec!["T1", "T2", "T3"],
        "each task ran exactly once"
    );
    o.final_digest().unwrap()
}

// ----------------------------------------------------------------- tests --

/// The exit criterion, part 1: the clean golden run delivers, the parallel
/// group is VISIBLE in the event stream (both T2/T3 `TaskStarted` before any
/// `TaskCompleted` of that pair), gates are audited, seq strictly increases,
/// ledger holds exactly one finished row per task.
#[test]
fn golden_clean_run_delivers_with_visible_parallel_group() {
    let dir = tempfile::tempdir().unwrap();
    let digest = clean_run(dir.path());
    assert_eq!(digest.len(), 64, "sha256 hex digest");

    let ev = read_events(dir.path());
    let kinds: Vec<&str> = ev.iter().map(|(_, k, _)| k.as_str()).collect();
    assert_eq!(
        kinds.first().unwrap(),
        &"TasksCreated",
        "mini-run opens the log"
    );
    assert_eq!(kinds.last().unwrap(), &"Delivered", "FSM event lands last");
    // seq strictly increasing within the run (§7.3 / P2).
    assert!(
        ev.windows(2).all(|w| w[1].0 > w[0].0),
        "seq must strictly increase: {ev:?}"
    );

    // The parallel pair: both starts precede the FIRST of their completions.
    let pos = |kind: &str, t: &str| {
        ev.iter()
            .position(|(_, k, tid)| k == kind && tid.as_deref() == Some(t))
            .unwrap()
    };
    assert!(
        pos("TaskStarted", "T2") < pos("TaskCompleted", "T2")
            && pos("TaskStarted", "T2") < pos("TaskStarted", "T3")
            && pos("TaskStarted", "T3") < pos("TaskCompleted", "T3"),
        "T2∥T3 group must be admitted together: {kinds:?}"
    );
    // both children started before either child COMPLETES: true parallelism
    assert!(
        pos("TaskStarted", "T3") < pos("TaskCompleted", "T2"),
        "T3 starts while T2 still open: {kinds:?}"
    );
    // gates audited (§9): every decision reached the event stream.
    assert_eq!(
        kinds
            .iter()
            .filter(|k| **k == "HumanApprovalGranted")
            .count(),
        2,
        "task_plan + delivery grants: {kinds:?}"
    );

    let store = StateStore::open(&dir.path().join(".ribozym").join("state.db")).unwrap();
    let rows: Vec<(String, String, i64)> = store
        .conn()
        .prepare("SELECT task_id, status, (SELECT COUNT(*) FROM ledger WHERE task_id = tasks.task_id) FROM tasks ORDER BY task_id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(|x| x.unwrap())
        .collect();
    assert_eq!(
        rows,
        vec![
            ("T1".into(), "done".into(), 1),
            ("T2".into(), "done".into(), 1),
            ("T3".into(), "done".into(), 1),
        ],
        "one ledger row per task, all done"
    );
    let gates: i64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM approvals", [], |r| r.get(0))
        .unwrap();
    assert_eq!(gates, 2, "both passed gates leave approval rows");
}

/// The exit criterion, part 2 (§21.2 kill→resume + v2.47 B4 ayrık-çökme
/// varyantı): interrupt the run AT EVERY discrete write step (and mid-work),
/// drop every handle (SIGKILL semantics), resume from disk ONLY — final
/// normalized hash identical to the clean run, and no task ever executes
/// twice across the kill boundary.
#[test]
fn golden_kill_resume_converges_from_every_write_window() {
    let reference = {
        let d = tempfile::tempdir().unwrap();
        clean_run(d.path())
    };
    let stages = [
        Stage::AfterStarted,   // window 1: ledger 'started' only
        Stage::AfterWork,      // window 2: result on disk, nothing committed
        Stage::AfterDoneTx,    // window 3: state.db done, no event, not finished
        Stage::AfterDoneEvent, // window 4: event written, ledger open
        Stage::AfterFinish,    // past the last write: clean resume, no recovery
    ];
    for stage in stages {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake::new();
        let mut o = Orchestrator::create(&cfg(dir.path()), &golden_plan()).unwrap();
        o.arm_crash(stage);
        let err = o.advance(&fake, true).unwrap_err();
        assert!(
            matches!(err, ribozym::Error::Crashed(_)),
            "{stage:?}: {err:?}"
        );
        drop(o); // the kill

        let fake2 = Fake::new();
        let mut r = Orchestrator::resume(&cfg(dir.path()), &golden_plan()).unwrap();
        let out = r.advance(&fake2, true).unwrap();
        assert_eq!(
            out,
            Outcome::Delivered,
            "resume from {stage:?} must deliver"
        );

        let mut calls = fake.calls();
        calls.extend(fake2.calls());
        for t in ["T1", "T2", "T3"] {
            let n = calls.iter().filter(|c| c.as_str() == t).count();
            assert_eq!(
                n, 1,
                "{t} ran {n}x across a kill at {stage:?} — §7.4 rule 4 violated"
            );
        }
        assert_eq!(
            r.final_digest().unwrap(),
            reference,
            "final hash drifted after kill at {stage:?}"
        );
    }
}

/// §7.4 agent_call recovery, isolated: kill after the result file is on disk
/// (`AfterWork` window). Resume's recovery phase must FINALIZE T1 without
/// any executor in sight — `resume` doesn't even take one; finalize happens
/// before `advance` runs. T2/T3 were never admitted, so they run fresh.
#[test]
fn resume_finalizes_persisted_result_without_touching_executor() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let mut o = Orchestrator::create(&cfg(dir.path()), &golden_plan()).unwrap();
    o.arm_crash(Stage::AfterWork);
    let _ = o.advance(&fake, true).unwrap_err();
    assert_eq!(fake.calls(), vec!["T1"]); // T1's work happened, then kill
    drop(o);

    let fake2 = Fake::new();
    let mut r = Orchestrator::resume(&cfg(dir.path()), &golden_plan()).unwrap();
    // T1 already Done BEFORE advance: recovery read the result file itself.
    let status: String = {
        let store = StateStore::open(&dir.path().join(".ribozym").join("state.db")).unwrap();
        store
            .conn()
            .query_row("SELECT status FROM tasks WHERE task_id='T1'", [], |x| {
                x.get(0)
            })
            .unwrap()
    };
    assert_eq!(status, "done", "resume recovery finalized T1");
    let out = r.advance(&fake2, true).unwrap();
    assert_eq!(out, Outcome::Delivered);
    assert_eq!(fake2.calls(), vec!["T2", "T3"], "T1 was never re-executed");
}

/// §9 interactive path, end to end: at L1 without --yes the task_plan gate
/// WAITS (nothing scheduled, pending row + request event on disk); a file
/// approval (`ribozym approve`'s table write) unblocks the RESUMED run; the
/// delivery gate then waits again (high-risk, L1) until its own approval.
#[test]
fn interactive_gates_wait_and_resume_passes_on_approval_rows() {
    let dir = tempfile::tempdir().unwrap();
    let c = RunConfig {
        batch_yes: false,
        ..cfg(dir.path())
    };
    let fake = Fake::new();
    let mut o = Orchestrator::create(&c, &golden_plan()).unwrap();
    let out = o.advance(&fake, true).unwrap();
    assert_eq!(
        out,
        Outcome::Waiting {
            gate: Gate::TaskPlan
        },
        "L1 gate waits for a human"
    );
    assert!(fake.calls().is_empty(), "no scheduling before the gate");
    drop(o);

    {
        let store = StateStore::open(&c.root.join(".ribozym").join("state.db")).unwrap();
        gates::Approvals::new(store.conn())
            .record(
                SPEC,
                Gate::TaskPlan,
                "user",
                "approved",
                Some("ok"),
                "2026-09-12T10:05:00Z",
            )
            .unwrap();
    }
    let mut r = Orchestrator::resume(&c, &golden_plan()).unwrap();
    let out = r.advance(&fake, true).unwrap();
    // plan approved → all 3 tasks run → delivery gate now waits.
    assert_eq!(
        out,
        Outcome::Waiting {
            gate: Gate::Delivery
        }
    );
    assert_eq!(
        fake.calls(),
        vec!["T1", "T2", "T3"],
        "scheduled work ran once the plan was approved"
    );
    // The request for delivery was PERSISTED once (timeout anchor survives).
    drop(r);
    {
        let store = StateStore::open(&c.root.join(".ribozym").join("state.db")).unwrap();
        let row = gates::Approvals::new(store.conn())
            .lookup(SPEC, Gate::Delivery)
            .unwrap()
            .expect("pending request row is durable");
        assert_eq!(row.decision, gates::decision::PENDING);
        gates::Approvals::new(store.conn())
            .record(
                SPEC,
                Gate::Delivery,
                "user",
                "approved",
                None,
                "2026-09-12T10:06:00Z",
            )
            .unwrap();
    }
    let mut r2 = Orchestrator::resume(&c, &golden_plan()).unwrap();
    let out = r2.advance(&Fake::new(), true).unwrap();
    assert_eq!(out, Outcome::Delivered);
    assert_eq!(
        fake.calls(),
        vec!["T1", "T2", "T3"],
        "delivered without re-executing anything"
    );
}

/// §9 Q4 low-risk timeout: a `pending` task_plan gate older than the window
/// auto-approves with a `timeout_auto` audit row on the NEXT process — the
/// clock anchor lives in the DB, not in memory. Delivery (high-risk) must
/// keep waiting instead of auto-approving.
#[test]
fn low_risk_timeout_auto_approves_delivery_parks() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    // t0: gate requested → waits, pending rows exist for task_plan.
    let mut o = Orchestrator::create(
        &RunConfig {
            batch_yes: false,
            ..cfg(dir.path())
        },
        &golden_plan(),
    )
    .unwrap();
    assert_eq!(
        o.advance(&fake, true).unwrap(),
        Outcome::Waiting {
            gate: Gate::TaskPlan
        }
    );
    drop(o);
    // t0+61, window 60: resume → task_plan auto-approves, delivery still waits.
    let late = RunConfig {
        batch_yes: false,
        gate_timeout_seconds: 60,
        now_epoch: 1_789_207_200 + 61,
        ..cfg(dir.path())
    };
    let mut r = Orchestrator::resume(&late, &golden_plan()).unwrap();
    let out = r.advance(&fake, true).unwrap();
    assert_eq!(
        out,
        Outcome::Waiting {
            gate: Gate::Delivery
        }
    );
    {
        let store = StateStore::open(&late.root.join(".ribozym").join("state.db")).unwrap();
        let a = gates::Approvals::new(store.conn());
        let row = a.lookup(SPEC, Gate::TaskPlan).unwrap().unwrap();
        assert_eq!(
            row.decision,
            gates::decision::TIMEOUT_AUTO,
            "Q4: low-risk auto on timeout"
        );
        assert_eq!(row.decided_by, "gate-timeout");
        // delivery's request happened on THIS process (its first poll, t0+61)
        let d = a.lookup(SPEC, Gate::Delivery).unwrap().unwrap();
        assert_eq!(d.ts, "2026-09-12T10:01:01Z");
    }
    assert_eq!(fake.calls(), vec!["T1", "T2", "T3"]);
    // Second poll at +120 (waited 59 < 60): still waiting AND the anchor must
    // not drift — re-stamping would make the §9 timeout unreachable.
    let still_short = RunConfig {
        now_epoch: 1_789_207_200 + 120,
        ..late.clone()
    };
    drop(r);
    let mut r1 = Orchestrator::resume(&still_short, &golden_plan()).unwrap();
    let fake_none = Fake::new();
    assert_eq!(
        r1.advance(&fake_none, true).unwrap(),
        Outcome::Waiting {
            gate: Gate::Delivery
        }
    );
    assert!(fake_none.calls().is_empty(), "a poll schedules no new work");
    {
        let store = StateStore::open(&still_short.root.join(".ribozym").join("state.db")).unwrap();
        let d = gates::Approvals::new(store.conn())
            .lookup(SPEC, Gate::Delivery)
            .unwrap()
            .unwrap();
        assert_eq!(d.decision, gates::decision::PENDING);
        assert_eq!(
            d.ts, "2026-09-12T10:01:01Z",
            "timeout anchor must survive polls"
        );
    }
    // Past the window: high-risk gate PARKS — NeedsUser, nothing lost (§9).
    let spent = RunConfig {
        now_epoch: 1_789_207_200 + 121,
        ..late.clone()
    };
    drop(r1);
    let mut r2 = Orchestrator::resume(&spent, &golden_plan()).unwrap();
    let out = r2.advance(&Fake::new(), true).unwrap();
    assert!(
        matches!(out, Outcome::NeedsUser { .. }),
        "high-risk gate parks: {out:?}"
    );
    // rule 4 after a park: the delivered-work tasks never re-ran, and a
    // terminal NeedsUser is stable across yet another resume.
    drop(r2);
    let mut r3 = Orchestrator::resume(&spent, &golden_plan()).unwrap();
    assert!(matches!(
        r3.advance(&Fake::new(), true).unwrap(),
        Outcome::NeedsUser { .. }
    ));
}

/// Double kill: crash once, resume, crash again at a DIFFERENT window,
/// resume again — the outcome still converges (recovery composes).
#[test]
fn double_kill_still_converges() {
    let reference = {
        let d = tempfile::tempdir().unwrap();
        clean_run(d.path())
    };
    let dir = tempfile::tempdir().unwrap();
    let c = cfg(dir.path());
    let mut calls_total = 0;

    let mut o = Orchestrator::create(&c, &golden_plan()).unwrap();
    o.arm_crash(Stage::AfterStarted); // T1's ledger row started only
    let fake = Fake::new();
    assert!(o
        .advance(&fake, true)
        .unwrap_err()
        .to_string()
        .contains("CRASH_SIMULATED"));
    calls_total += fake.calls().len();
    drop(o);

    let mut r = Orchestrator::resume(&c, &golden_plan()).unwrap();
    r.arm_crash(Stage::AfterDoneEvent); // now kill mid-COMMIT of T1
    let fake2 = Fake::new();
    assert!(r
        .advance(&fake2, true)
        .unwrap_err()
        .to_string()
        .contains("CRASH_SIMULATED"));
    calls_total += fake2.calls().len();
    drop(r);

    let mut r2 = Orchestrator::resume(&c, &golden_plan()).unwrap();
    let fake3 = Fake::new();
    assert_eq!(r2.advance(&fake3, true).unwrap(), Outcome::Delivered);
    calls_total += fake3.calls().len();
    assert_eq!(
        calls_total, 3,
        "each task executed exactly once through two kills"
    );
    assert_eq!(r2.final_digest().unwrap(), reference);
}

// ------------------------------------------------ review-fix regressions --

/// Review finding: a finished ledger op must be the LAST word (§7.4 rule 4).
/// Simulate the one path the phase machine cannot see — a misused second
/// `create` that resets `tasks` to pending over a spec whose ledger already
/// says everything is done. The run must converge WITHOUT executing anything
/// and WITHOUT changing the final hash.
#[test]
fn finished_ops_never_re_run_even_after_misused_recreate() {
    let dir = tempfile::tempdir().unwrap();
    let digest = clean_run(dir.path());
    // Misuse: create AGAIN on the same spec dir (rows reset, run status resets).
    let fake2 = Fake::new();
    let mut o2 = Orchestrator::create(&cfg(dir.path()), &golden_plan()).unwrap();
    assert_eq!(o2.advance(&fake2, true).unwrap(), Outcome::Delivered);
    assert!(
        fake2.calls().is_empty(),
        "rule 4: zero re-execution, got {:?}",
        fake2.calls()
    );
    assert_eq!(
        o2.final_digest().unwrap(),
        digest,
        "misuse must not shift the hash"
    );
}

/// A failed executor: no success result is ever persisted, the DAG cascades
/// to `blocked`, and the run terminates FAILED with `runs.status` synced to
/// the terminal phase (a dangling `scheduling` row would make a resume spin).
struct FailAll;
impl TaskExecutor for FailAll {
    fn execute(&self, task: &TaskItem, _op: &str) -> ribozym::Result<Executed> {
        Ok(Executed {
            summary: json!({"task": task.id, "error": "boom"}),
            ok: false,
            cost_usd: 0.0,
            budget_denied: false,
        })
    }
}

#[test]
fn all_failed_cascades_blocks_and_persists_terminal_phase() {
    let dir = tempfile::tempdir().unwrap();
    let mut o = Orchestrator::create(&cfg(dir.path()), &golden_plan()).unwrap();
    let out = o.advance(&FailAll, true).unwrap();
    assert!(
        matches!(out, Outcome::Failed { .. }),
        "no done ⇒ Failed: {out:?}"
    );
    let store = StateStore::open(&dir.path().join(".ribozym").join("state.db")).unwrap();
    let status: String = store
        .conn()
        .query_row(
            "SELECT status FROM runs WHERE spec_id = 'spec-20260912-golden'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        status, "failed",
        "terminal phase must be persisted, not left at scheduling"
    );
    let rows: Vec<(String, String)> = store
        .conn()
        .prepare("SELECT task_id, status FROM tasks ORDER BY task_id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(|x| x.unwrap())
        .collect();
    assert_eq!(
        rows,
        vec![
            ("T1".into(), "failed".into()),
            ("T2".into(), "blocked".into()),
            ("T3".into(), "blocked".into()),
        ],
        "failure must cascade to dependents"
    );
    // No result files for failed ops (they carry no result to finalize).
    assert!(
        !dir.path()
            .join("runs")
            .join(SPEC)
            .join("artifacts")
            .exists(),
        "failed ops must not persist a result_ref"
    );
}
