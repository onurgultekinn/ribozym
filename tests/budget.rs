//! Faz 4-debt regressions: the §8.2 budget clamp (soft warning + hard wall →
//! Paused → resume) and the §6.4 traceability gate, exercised through the live
//! orchestrator so neither is a decorative/untested path. Deterministic, no
//! LLM, injected clock + injected cost (§21.1).

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use ribozym::models::spec::Spec;
use ribozym::models::task::{Acceptance, Estimate, TaskItem, TaskPlan};
use ribozym::orchestrator::{Executed, Orchestrator, Outcome, RunConfig, Stage, TaskExecutor};
use ribozym::state::StateStore;
use serde_json::json;

// ------------------------------------------------------------------ fixtures

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn task_costed(id: &str, deps: &[&str], cost_usd: f64) -> TaskItem {
    let _ = repo_root();
    TaskItem {
        id: id.into(),
        spec_requirement_ids: vec!["R1".into()],
        title: format!("costed {id}"),
        description: String::new(),
        department: None,
        assigned_agent: "A5".into(),
        dependencies: deps.iter().map(|s| s.to_string()).collect(),
        target: "cloud".into(),
        toolchain: None,
        acceptance: Acceptance {
            verifiable_by: "A4".into(),
            criteria: "done".into(),
        },
        // DECLARED estimate: this is what the run-level pre-clamp reserves
        // against the hard wall (§8.2). Real cost is reported post-execution.
        estimated: Some(Estimate {
            tokens_in: 1000,
            tokens_out: 500,
            cost_usd,
            duration_seconds: 30,
        }),
        difficulty: None,
        file_ownership: vec![format!("src/{id}.rs")],
        artifacts_expected: vec![format!("src/{id}.rs")],
        artifacts_produced: vec![],
        blocked_by: vec![],
        status: "pending".into(),
        retry_count: 0,
        escalation_level: 0,
        notes: String::new(),
        extra: Default::default(),
    }
}

fn linear_plan(cost_each: f64) -> TaskPlan {
    TaskPlan {
        schema_version: "1.0".into(),
        spec_id: SPEC.into(),
        created_at: "2026-09-12T00:00:00Z".into(),
        created_by: "A2".into(),
        revision: 1,
        tasks: vec![
            task_costed("T1", &[], cost_each),
            task_costed("T2", &["T1"], cost_each),
            task_costed("T3", &["T2"], cost_each),
        ],
        execution_order: vec!["T1".into(), "T2".into(), "T3".into()],
        parallel_groups: vec![],
        critical_path: vec!["T1".into(), "T2".into(), "T3".into()],
        status_summary: Default::default(),
        extra: Default::default(),
    }
}

const SPEC: &str = "spec-20260912-budget";

fn cfg_with(root: &Path, budget_usd: f64) -> RunConfig {
    RunConfig {
        root: root.to_path_buf(),
        spec_id: SPEC.into(),
        run_id: "run-budget".into(),
        autonomy_level: 3, // auto gates; isolate the budget path from §9
        batch_yes: true,
        max_concurrency: 1,
        crisis_max: 2,
        gate_timeout_seconds: 1200,
        budget_usd,
        budget_soft_pct: 80.0,
        budget_hard_pct: 100.0,
        now_epoch: 1_789_207_200,
    }
}

/// Reports a REAL cost on completion — the driver's clamp and spent ledger
/// must react to it (the MockExecutor's 0.0 would prove nothing here).
struct Costed {
    per_task: f64,
    calls: RefCell<Vec<String>>,
}
impl TaskExecutor for Costed {
    fn execute(&self, task: &TaskItem, _op: &str) -> ribozym::Result<Executed> {
        self.calls.borrow_mut().push(task.id.clone());
        Ok(Executed {
            summary: json!({"task": task.id, "spent": self.per_task}),
            ok: true,
            cost_usd: self.per_task,
            budget_denied: false,
        })
    }
}

// -------------------------------------------------------------------- tests

/// §8.2 hard wall: with a budget that fits exactly two tasks, the third is
/// refused BEFORE it starts (pre-clamp reserves the declared estimate), the
/// run pauses, and the two completed tasks + their spend are preserved.
#[test]
fn hard_budget_wall_pauses_before_the_third_task_and_keeps_completed_work() {
    let dir = tempfile::tempdir().unwrap();
    let per = 1.0;
    // budget = 2.0 → T1, T2 fit (spent 2.0); T3's estimate pushes over hard.
    let c = cfg_with(dir.path(), 2.0);
    let fake = Costed {
        per_task: per,
        calls: RefCell::new(vec![]),
    };
    let mut o = Orchestrator::create(&c, &linear_plan(per)).unwrap();
    let out = o.advance(&fake, true).unwrap();
    assert!(
        matches!(out, Outcome::Paused { .. }),
        "expected Paused: {out:?}"
    );
    assert_eq!(
        fake.calls.borrow().clone(),
        vec!["T1", "T2"],
        "T3 must never start once the wall is hit"
    );
    // spent_usd accumulated truthfully = 2.0 (two real executions).
    assert!(
        (o.spent_usd() - 2.0).abs() < 1e-9,
        "spent {}",
        o.spent_usd()
    );
    // runs.spent_usd persisted (not a decorative column).
    let store = StateStore::open(&dir.path().join(".ribozym").join("state.db")).unwrap();
    let spent: f64 = store
        .conn()
        .query_row(
            "SELECT spent_usd FROM runs WHERE spec_id = ?1",
            [SPEC],
            |r| r.get(0),
        )
        .unwrap();
    assert!((spent - 2.0).abs() < 1e-9, "persisted spent {spent}");
    let status: String = store
        .conn()
        .query_row("SELECT status FROM runs WHERE spec_id = ?1", [SPEC], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(status, "paused");
    let pf: Option<String> = store
        .conn()
        .query_row(
            "SELECT paused_from FROM runs WHERE spec_id = ?1",
            [SPEC],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        pf.as_deref(),
        Some("scheduling"),
        "resumable anchor persisted"
    );
    let done: i64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM tasks WHERE status='done'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(done, 2, "completed work preserved for resume");
}

/// §8.2/B3 recovery: raise the budget and resume — the run continues from
/// `paused_from`, finishes T3, delivers, and NO task executes twice (T1/T2
/// stay `done` via ledger rule 4; only T3 runs).
#[test]
fn raised_budget_resumes_from_pause_and_delivers() {
    let dir = tempfile::tempdir().unwrap();
    let per = 1.0;
    let fake = Costed {
        per_task: per,
        calls: RefCell::new(vec![]),
    };
    let c = cfg_with(dir.path(), 2.0);
    let mut o = Orchestrator::create(&c, &linear_plan(per)).unwrap();
    assert!(matches!(
        o.advance(&fake, true).unwrap(),
        Outcome::Paused { .. }
    ));
    assert_eq!(fake.calls.borrow().len(), 2);
    drop(o);

    // Budget raised to 5.0 — resume + unpause + advance.
    let bigger = cfg_with(dir.path(), 5.0);
    let mut r = Orchestrator::resume(&bigger, &linear_plan(per)).unwrap();
    assert!(
        r.unpause().unwrap(),
        "run was paused → unpause fires Resume"
    );
    assert_eq!(
        r.phase(),
        ribozym::fsm::Phase::Scheduling,
        "resumed into work"
    );
    let out = r.advance(&fake, true).unwrap();
    assert_eq!(out, Outcome::Delivered, "raised budget lets T3 finish");
    assert_eq!(
        fake.calls.borrow().clone(),
        vec!["T1", "T2", "T3"],
        "T1/T2 must NOT re-run after resume"
    );
    assert!(
        (r.spent_usd() - 3.0).abs() < 1e-9,
        "spent {}",
        r.spent_usd()
    );
}

/// §8.2 soft limit: crossing 80% fires exactly ONE BudgetWarning event even
/// when the run then pauses above the hard wall (the `budget_warned` latch
/// must not re-warn on the same run).
#[test]
fn soft_limit_warns_exactly_once() {
    let dir = tempfile::tempdir().unwrap();
    // 3 chained tasks, each estimating 0.9. budget 2.0 ⇒ soft wall = 1.6.
    let plan = linear_plan(0.9);
    let c = cfg_with(dir.path(), 2.0);
    // Executor reports 0.8 real cost per task: T1(0.8)+T2(0.8)=1.6 crosses
    // soft exactly; T3's 0.9 reserve breaches hard ⇒ pause.
    let fake = Costed {
        per_task: 0.8,
        calls: RefCell::new(vec![]),
    };
    let mut o = Orchestrator::create(&c, &plan).unwrap();
    let out = o.advance(&fake, true).unwrap();
    assert!(
        matches!(out, Outcome::Paused { .. }),
        "expected pause: {out:?}"
    );
    assert_eq!(*fake.calls.borrow(), vec!["T1", "T2"]);
    let events =
        std::fs::read_to_string(dir.path().join("runs").join(SPEC).join("events.jsonl")).unwrap();
    let warns = events.matches("\"BudgetWarning\"").count();
    assert_eq!(warns, 1, "soft limit fires exactly once, got {warns}");
    // Resume under a bigger budget: the latch is PERSISTED, so finishing T3
    // (spend crosses 2.4 > soft) must NOT warn a second time.
    drop(o);
    let bigger = cfg_with(dir.path(), 5.0);
    let mut r = Orchestrator::resume(&bigger, &plan).unwrap();
    assert!(r.unpause().unwrap());
    assert_eq!(r.advance(&fake, true).unwrap(), Outcome::Delivered);
    let events2 =
        std::fs::read_to_string(dir.path().join("runs").join(SPEC).join("events.jsonl")).unwrap();
    assert_eq!(
        events2.matches("\"BudgetWarning\"").count(),
        1,
        "latch survived the pause→resume boundary"
    );
}

/// §6.4 traceability gate through the live create: an uncovered requirement
/// is refused with NO run row written; a fully covered spec+plan creates.
#[test]
fn traceability_gate_blocks_create_and_passes_when_covered() {
    let dir = tempfile::tempdir().unwrap();
    let spec: Spec = {
        let p = repo_root().join("examples/expected/spec_valid.json");
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    };
    // spec_valid declares R1,R2. Make a plan that covers only R1 — bound to
    // the spec's own id so the SPEC_ID_MISMATCH guard is orthogonal (tested
    // separately below).
    let mut plan = linear_plan(0.0);
    plan.spec_id = spec.id.clone();
    let c = RunConfig {
        spec_id: spec.id.clone(),
        ..cfg_with(dir.path(), 5.0)
    };
    for t in &mut plan.tasks {
        t.spec_requirement_ids = vec!["R1".into()]; // R2 never cited
    }
    let err = Orchestrator::create_with_spec(&c, &plan, Some(&spec)).unwrap_err();
    assert!(
        matches!(err, ribozym::Error::Traceability(_)),
        "uncovered spec must be refused, got {err:?}"
    );
    assert!(err.to_string().contains("R2"), "{err}");
    assert!(
        !err.to_string().contains("SPEC_ID_MISMATCH"),
        "id must match here: {err}"
    );
    // The refusal must have written NOTHING (no half-persisted bad run).
    let exists_now = Orchestrator::exists(&c).unwrap();
    assert!(!exists_now, "a rejected plan must not create a runs row");
    // Fix the coverage → create succeeds.
    plan.tasks[0].spec_requirement_ids.push("R2".into());
    Orchestrator::create_with_spec(&c, &plan, Some(&spec)).unwrap();
    assert!(Orchestrator::exists(&c).unwrap());
}

/// The SPEC_ID_MISMATCH branch of the gate: a plan bound to a different spec
/// is refused even when every requirement name happens to be covered — a
/// mislabeled plan must never pass under the right id.
#[test]
fn traceability_gate_catches_spec_id_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let spec: Spec = {
        let p = repo_root().join("examples/expected/spec_valid.json");
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    };
    let mut plan = linear_plan(0.0); // spec-20260912-budget id
                                     // cover both req names so the ONLY problem is the id mismatch
    plan.tasks[0].spec_requirement_ids = vec!["R1".into(), "R2".into()];
    let c = cfg_with(dir.path(), 5.0);
    let err = Orchestrator::create_with_spec(&c, &plan, Some(&spec)).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("SPEC_ID_MISMATCH"), "{msg}");
    assert!(
        !msg.contains("TASK_UNCOVERED"),
        "coverage is complete here: {msg}"
    );
}

/// Sanity: the same plan with NO spec skips the cross-doc gate (honest,
/// reported) rather than pretending to have checked it.
#[test]
fn no_spec_supplied_skips_cross_doc_gate() {
    let dir = tempfile::tempdir().unwrap();
    let mut plan = linear_plan(0.0);
    for t in &mut plan.tasks {
        t.spec_requirement_ids = vec![]; // would fail coverage if a spec were given
    }
    let c = cfg_with(dir.path(), 5.0);
    // No spec → no cross-doc check → create is allowed (schema-level rules are
    // enforced elsewhere, at A2 output validation).
    Orchestrator::create_with_spec(&c, &plan, None).unwrap();
}

// ---------------------------------------------- Faz 5-d charge regressions --

/// §8.2/§7.4 regression (B4 window): a kill AFTER the spend-commit but
/// BEFORE the ledger close used to DOUBLE-CHARGE on resume — the
/// FinalizeResult branch re-booked the persisted cost on top of a row that
/// already carried it (the mirror-based absolute write silently rebased).
/// The `charged_at` guard (migration 0003) makes the increment exactly-once.
#[test]
fn kill_in_commit_window_charges_exactly_once_across_resume() {
    let dir = tempfile::tempdir().unwrap();
    let c = cfg_with(dir.path(), 10.0);
    let fake = Costed {
        per_task: 1.0,
        calls: RefCell::new(vec![]),
    };
    let mut o = Orchestrator::create(&c, &linear_plan(1.0)).unwrap();
    o.arm_crash(Stage::AfterDoneTx); // the spend tx committed, ledger close didn't
    let _ = o.advance(&fake, true).unwrap_err();
    assert_eq!(*fake.calls.borrow(), vec!["T1"]);
    drop(o);

    let mut r = Orchestrator::resume(&c, &linear_plan(1.0)).unwrap();
    let fake2 = Costed {
        per_task: 1.0,
        calls: RefCell::new(vec![]),
    };
    assert_eq!(r.advance(&fake2, true).unwrap(), Outcome::Delivered);
    assert_eq!(
        *fake2.calls.borrow(),
        vec!["T2", "T3"],
        "T1 never re-executed"
    );
    // 3 tasks × $1.0 — NOT 4.0 (T1's recovery re-booking the charge).
    assert!(
        (r.spent_usd() - 3.0).abs() < 1e-9,
        "resume must charge each op exactly once, got {}",
        r.spent_usd()
    );
    // The guard column itself is the durable proof.
    let store = StateStore::open(&dir.path().join(".ribozym").join("state.db")).unwrap();
    let n: i64 = store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM ledger WHERE charged_at IS NOT NULL",
            [],
            |x| x.get(0),
        )
        .unwrap();
    assert_eq!(n, 3, "every op carries exactly one charge stamp");
}

/// The soft-limit latch is STICKY across processes: a later non-crossing
/// commit must not rewrite `budget_warned` back to 0 — the old code wrote
/// `crosses_soft as i64` unconditionally, so one clear+reload cycle re-armed
/// the warning and the user would see BudgetWarning twice for the same run.
#[test]
fn soft_latch_is_sticky_across_process_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    // 8 tasks × $1; budget $10, soft 60% ⇒ the warning crosses at T6 ($6).
    let mut c = cfg_with(dir.path(), 10.0);
    c.budget_soft_pct = 60.0;
    let plan8 = || -> TaskPlan {
        let mut p = linear_plan(1.0);
        for i in 4..=8 {
            let dep = format!("T{}", i - 1);
            p.tasks
                .push(task_costed(&format!("T{i}"), &[dep.as_str()], 1.0));
        }
        p
    };
    let evpath = dir.path().join("runs").join(SPEC).join("events.jsonl");

    let p1 = Costed {
        per_task: 1.0,
        calls: RefCell::new(vec![]),
    };
    let mut o = Orchestrator::create(&c, &plan8()).unwrap();
    o.arm_crash(Stage::AfterStarted); // kill the SEVENTH task right as it starts
    let _ = o.advance(&p1, true).unwrap_err();
    drop(o);

    let p2 = Costed {
        per_task: 1.0,
        calls: RefCell::new(vec![]),
    };
    let mut r = Orchestrator::resume(&c, &plan8()).unwrap();
    r.arm_crash(Stage::AfterStarted); // kill the EIGHTH start too
    let _ = r.advance(&p2, true).unwrap_err();
    drop(r);

    let p3 = Costed {
        per_task: 1.0,
        calls: RefCell::new(vec![]),
    };
    let mut t = Orchestrator::resume(&c, &plan8()).unwrap();
    assert_eq!(t.advance(&p3, true).unwrap(), Outcome::Delivered);
    // 8 × $1.0 exactly.
    assert!(
        (t.spent_usd() - 8.0).abs() < 1e-9,
        "spent {}",
        t.spent_usd()
    );
    let events = std::fs::read_to_string(&evpath).unwrap();
    assert_eq!(
        events.matches("\"BudgetWarning\"").count(),
        1,
        "one warning per run, ever — a cleared latch would re-fire it here"
    );
}

/// §8.2/B3: a wall hit at the CALL boundary pauses the run — it does not
/// fail the work. Completed tasks stay done, the refused task goes back to
/// `pending` (no blocked cascade — the ladder is for broken work, not empty
/// wallets), and a budget raise + resume continues exactly where it stopped.
#[test]
fn wall_hit_at_call_boundary_pauses_then_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let c = cfg_with(dir.path(), 10.0);

    struct Wall {
        deny_after: usize,
        calls: std::cell::Cell<usize>,
    }
    impl TaskExecutor for Wall {
        fn execute(&self, task: &TaskItem, _op: &str) -> ribozym::Result<Executed> {
            self.calls.set(self.calls.get() + 1);
            if self.calls.get() > self.deny_after {
                return Ok(Executed {
                    summary: json!({"task": task.id, "error": "MODEL_BUDGET_DENIED"}),
                    ok: false,
                    cost_usd: 0.0,
                    budget_denied: true,
                });
            }
            Ok(Executed {
                summary: json!({"task": task.id}),
                ok: true,
                cost_usd: 1.0,
                budget_denied: false,
            })
        }
    }

    let wall = Wall {
        deny_after: 2,
        calls: std::cell::Cell::new(0),
    };
    let mut o = Orchestrator::create(&c, &linear_plan(1.0)).unwrap();
    let out = o.advance(&wall, true).unwrap();
    assert!(matches!(out, Outcome::Paused { .. }), "got {out:?}");

    let store = StateStore::open(&dir.path().join(".ribozym").join("state.db")).unwrap();
    let statuses: Vec<(String, String)> = {
        let mut st = store
            .conn()
            .prepare("SELECT task_id, status FROM tasks ORDER BY task_id")
            .unwrap();
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert_eq!(
        statuses,
        vec![
            ("T1".to_string(), "done".to_string()),
            ("T2".to_string(), "done".to_string()),
            ("T3".to_string(), "pending".to_string()),
        ],
        "finished work kept; refused task pending again, NOT failed/blocked"
    );
    let (run_status, paused_from): (String, Option<String>) = store
        .conn()
        .query_row("SELECT status, paused_from FROM runs", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(
        (run_status.as_str(), paused_from.as_deref()),
        ("paused", Some("scheduling"))
    );
    // Spend of the completed work survived the pause.
    let spent: f64 = store
        .conn()
        .query_row("SELECT spent_usd FROM runs", [], |r| r.get(0))
        .unwrap();
    assert!((spent - 2.0).abs() < 1e-9, "spent {spent}");
    drop(store);

    // Budget raised (fresh config, same wall): resume continues T3 — and only T3.
    let mut c2 = cfg_with(dir.path(), 20.0);
    c2.run_id = "run-resume".into();
    let mut r = Orchestrator::resume(&c2, &linear_plan(1.0)).unwrap();
    assert!(r.unpause().unwrap(), "paused run must resume");
    let wall2 = Wall {
        deny_after: 99,
        calls: std::cell::Cell::new(0),
    };
    assert_eq!(r.advance(&wall2, true).unwrap(), Outcome::Delivered);
    assert_eq!(wall2.calls.get(), 1, "T1/T2 never re-ran; only T3 executed");
}
