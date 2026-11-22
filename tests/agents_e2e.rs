//! Faz 5-a end-to-end: the A5 coder agent inside the REAL orchestrator loop.
//! The LLM is a scripted MockProvider (§21.1) but everything else is real:
//! SchemaGuard (full jsonschema), the ToolGateway (full-argv allowlist, real
//! `cargo` execution via SystemRunner), the §7.4 ledger windows, the budget
//! accrual, and the golden digest. "cargo check" verifies a real Rust file.

use std::cell::RefCell;
use std::path::Path;
use std::sync::Arc;

use ribozym::agents::AgentBridge;
use ribozym::config;
use ribozym::gateway::ToolGateway;
use ribozym::inference::{MockProvider, MockStep};
use ribozym::models::task::{Acceptance, TaskItem, TaskPlan};
use ribozym::orchestrator::{Executed, Orchestrator, Outcome, RunConfig, TaskExecutor};

const SPEC: &str = "spec-20260912-agent";

// ------------------------------------------------------------------ helpers

fn valid_coding_output(task_id: &str, code: &str) -> String {
    serde_json::json!({
        "schema_version": "1.0",
        "task_id": task_id,
        "files": [{ "path": "src/lib.rs", "content": code }],
        "self_check": { "commands": ["cargo check"] },
        "notes": "scaffold"
    })
    .to_string()
}

/// A no-op Rust file — `cargo check` passes trivially once a Cargo.toml exists.
const GOOD_CODE: &str = "pub fn alive() -> bool { true }\n";

/// A workdir pre-seeded with a minimal compilable crate.
fn seed_crate(dir: &Path) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"golden-agent\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("src").join("lib.rs"), "// seed\n").unwrap();
}

fn task(id: &str, deps: &[&str], cost: f64) -> TaskItem {
    TaskItem {
        id: id.into(),
        spec_requirement_ids: vec!["R1".into()],
        title: format!("write {id}"),
        description: "produce src/lib.rs".into(),
        department: None,
        assigned_agent: "A5".into(),
        dependencies: deps.iter().map(|s| s.to_string()).collect(),
        target: "cloud".into(),
        toolchain: Some("rust".into()),
        acceptance: Acceptance {
            verifiable_by: "cargo check".into(),
            criteria: "cargo check exits 0".into(),
        },
        estimated: Some(ribozym::models::task::Estimate {
            tokens_in: 1000,
            tokens_out: 500,
            cost_usd: cost,
            duration_seconds: 30,
        }),
        difficulty: None,
        file_ownership: vec!["src/lib.rs".into()],
        artifacts_expected: vec!["src/lib.rs".into()],
        artifacts_produced: vec![],
        blocked_by: vec![],
        status: "pending".into(),
        retry_count: 0,
        escalation_level: 0,
        notes: String::new(),
        extra: Default::default(),
    }
}

fn plan() -> TaskPlan {
    TaskPlan {
        schema_version: "1.0".into(),
        spec_id: SPEC.into(),
        created_at: "2026-09-12T00:00:00Z".into(),
        created_by: "A2".into(),
        revision: 1,
        tasks: vec![task("T1", &[], 0.001), task("T2", &["T1"], 0.001)],
        execution_order: vec!["T1".into(), "T2".into()],
        parallel_groups: vec![],
        critical_path: vec!["T1".into(), "T2".into()],
        status_summary: Default::default(),
        extra: Default::default(),
    }
}

/// Real rust toolchain gateway: the ribozym.toml allowlist governs, the
/// SystemRunner really spawns. Workdir = the seeded crate copy.
fn gateway(workdir: &Path) -> Arc<ToolGateway> {
    let root = config::find_repo_root(&std::env::current_dir().unwrap()).unwrap();
    let project = config::load_project_config(&root).unwrap();
    Arc::new(ToolGateway::new(
        project.toolchains,
        workdir.to_path_buf(),
        Arc::new(ribozym::gateway::SystemRunner),
        std::env::vars().collect(),
    ))
}

fn cfg(root: &Path) -> RunConfig {
    RunConfig {
        root: root.to_path_buf(),
        spec_id: SPEC.into(),
        run_id: "run-agent".into(),
        autonomy_level: 3,
        batch_yes: true,
        max_concurrency: 1,
        crisis_max: 2,
        gate_timeout_seconds: 1200,
        budget_usd: 5.0,
        budget_soft_pct: 80.0,
        budget_hard_pct: 100.0,
        now_epoch: 1_789_207_200,
    }
}

/// Bridge executor that ALSO records per-task LLM turns for assertions.
struct Bridged {
    inner: AgentBridge,
    calls: RefCell<Vec<String>>,
}
impl TaskExecutor for Bridged {
    fn execute(&self, task: &TaskItem, op_id: &str) -> ribozym::Result<Executed> {
        self.calls.borrow_mut().push(task.id.clone());
        self.inner.execute(task, op_id)
    }
}

// -------------------------------------------------------------------- tests

/// The full happy path: two chained tasks, each an LLM turn whose patch is
/// written into the isolated workdir and verified by a REAL `cargo check`.
/// Spent accrues from ModelUsage, ledger rows close, run delivers.
#[test]
fn a5_agent_pipeline_writes_and_verifies_real_code() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path().join("workdir");
    seed_crate(&workdir);

    let mock = Arc::new(MockProvider::new(vec![
        MockStep::Success {
            text: valid_coding_output("T1", GOOD_CODE),
            tokens_in: 500,
            tokens_out: 200,
        },
        MockStep::Success {
            text: valid_coding_output("T2", GOOD_CODE),
            tokens_in: 500,
            tokens_out: 200,
        },
    ]));
    let bridge = AgentBridge::new(
        config::find_repo_root(&std::env::current_dir().unwrap()).unwrap(),
        mock.clone(),
        gateway(&workdir),
        &workdir,
        "ADR: axum service; write src/lib.rs only",
    );
    let exec = Bridged {
        inner: bridge,
        calls: RefCell::new(vec![]),
    };

    let mut o = Orchestrator::create(&cfg(dir.path()), &plan()).unwrap();
    let out = o.advance(&exec, true).unwrap();
    assert_eq!(out, Outcome::Delivered);
    assert_eq!(*exec.calls.borrow(), vec!["T1", "T2"]);

    // The model's file REALLY landed in the workdir with the model's bytes.
    let written = std::fs::read_to_string(workdir.join("src").join("lib.rs")).unwrap();
    assert_eq!(written, GOOD_CODE);

    // The mock engine bills $0 honestly; what must hold is that spent_usd
    // came from ModelUsage (0.0 from usage, not the tasks' 0.001 estimates:
    // spent must differ from the 2×0.001 estimate-reserve if any real path
    // fed it). Assert BOTH: mock cost 0 AND spent != estimate sum.
    assert_eq!(o.spent_usd(), 0.0, "mock engine bills nothing");
    assert!(
        (o.spent_usd() - 0.002).abs() > 1e-9,
        "spent must come from ModelUsage, NOT the declared estimates"
    );
    // Ledger: one closed agent_call op per task.
    let store =
        ribozym::state::StateStore::open(&dir.path().join(".ribozym").join("state.db")).unwrap();
    let (n, closed): (i64, i64) = store
        .conn()
        .query_row(
            "SELECT COUNT(*), SUM(state='finished') FROM ledger WHERE spec_id = ?1",
            [SPEC],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((n, closed), (2, 2), "both agent ops finished");
}

/// §14 repair: a prose-wrapped garbage first response forces the repair turn;
/// the second scripted response is clean and the run still delivers.
#[test]
fn a5_repairs_schema_invalid_first_response() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path().join("workdir");
    seed_crate(&workdir);

    let bad = "Sure, here is my plan: {\"use\": \"the format below\"} \
               {\"schema_version\":\"1.0\",\"task_id\":\"T1\",\"files\":[],\"self_check\":{\"commands\":[]}}";
    let mock = Arc::new(MockProvider::new(vec![
        MockStep::Success {
            text: bad.into(),
            tokens_in: 100,
            tokens_out: 50,
        },
        MockStep::Success {
            text: valid_coding_output("T1", GOOD_CODE),
            tokens_in: 100,
            tokens_out: 50,
        },
        MockStep::Success {
            text: valid_coding_output("T2", GOOD_CODE),
            tokens_in: 100,
            tokens_out: 50,
        },
    ]));
    let bridge = AgentBridge::new(
        config::find_repo_root(&std::env::current_dir().unwrap()).unwrap(),
        mock.clone(),
        gateway(&workdir),
        &workdir,
        "ctx",
    );
    let exec = Bridged {
        inner: bridge,
        calls: RefCell::new(vec![]),
    };
    let mut o = Orchestrator::create(&cfg(dir.path()), &plan()).unwrap();
    assert_eq!(o.advance(&exec, true).unwrap(), Outcome::Delivered);
    // 2 tasks, but 3 LLM calls total (T1 needed the repair turn).
    assert_eq!(mock.calls.lock().unwrap().len(), 3);
}

/// §8.1 E1 fix loop: a model whose code never compiles is retried up to the
/// cap, then the task FAILS (TaskFailed), its dependent blocks, run fails —
/// with runs.status persisted terminal (no dangling scheduling).
#[test]
fn a5_fix_loop_exhausts_and_fails_the_run_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path().join("workdir");
    seed_crate(&workdir);

    // Script: 4 broken-code responses (initial + 3 fix-loop) for T1, all
    // producing a file with a syntax error; cargo check keeps failing.
    let broken = "this is not rust <<<>>>";
    let steps: Vec<MockStep> = (0..3)
        .map(|_| MockStep::Success {
            text: valid_coding_output("T1", broken),
            tokens_in: 100,
            tokens_out: 50,
        })
        .collect();
    let mock = Arc::new(MockProvider::new(steps));
    let bridge = AgentBridge::new(
        config::find_repo_root(&std::env::current_dir().unwrap()).unwrap(),
        mock.clone(),
        gateway(&workdir),
        &workdir,
        "ctx",
    );
    let exec = Bridged {
        inner: bridge,
        calls: RefCell::new(vec![]),
    };
    let mut o = Orchestrator::create(&cfg(dir.path()), &plan()).unwrap();
    let out = o.advance(&exec, true).unwrap();
    assert!(matches!(out, Outcome::Failed { .. }), "got {out:?}");
    // 3 LLM calls = initial + 2 fix iterations (FIX_LOOP_MAX=3 TOTAL turns,
    // not 3 retries after the first — §8.1 E1's ceiling is on TOTAL attempts).
    assert_eq!(mock.calls.lock().unwrap().len(), 3);
    // Terminal state persisted.
    let store =
        ribozym::state::StateStore::open(&dir.path().join(".ribozym").join("state.db")).unwrap();
    let status: String = store
        .conn()
        .query_row("SELECT status FROM runs WHERE spec_id = ?1", [SPEC], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(status, "failed");
    let t2: String = store
        .conn()
        .query_row("SELECT status FROM tasks WHERE task_id = 'T2'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(t2, "blocked", "dependent task never ran");
}

/// The tool gateway really spawns cargo here: assert a `cargo check` run
/// against the seeded crate passes through the allowlist with real output.
#[test]
fn gateway_runs_real_cargo_check_in_workdir() {
    let dir = tempfile::tempdir().unwrap();
    seed_crate(dir.path());
    let g = gateway(dir.path());
    let out = g.run("cargo check").expect("allowlisted");
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    // Also prove the allowlist denies arbitrary commands outright.
    assert!(matches!(
        g.check("cargo publish"),
        ribozym::gateway::Check::Denied { .. }
    ));
}
