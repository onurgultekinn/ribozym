//! Faz 5-b end-to-end: the §6.3 code pipeline as REAL agents through the REAL
//! orchestrator — A5 (code) → A7 (security) → A4 (tests). LLM is scripted
//! (§21.1); gates, paths, gateway, ledger, budget are all real. cargo really
//! compiles and really runs the tests A4 writes.

use std::cell::RefCell;
use std::path::Path;

use ribozym::agents::AgentBridge;
use ribozym::config;
use ribozym::gateway::ToolGateway;
use ribozym::inference::{MockProvider, MockStep};
use ribozym::models::task::{Acceptance, Estimate, TaskItem, TaskPlan};
use ribozym::orchestrator::{Executed, Orchestrator, Outcome, RunConfig, TaskExecutor};

const SPEC: &str = "spec-20260912-pipe";

fn seed_crate(dir: &Path) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"pipe-agent\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("src").join("lib.rs"), "// seed\n").unwrap();
}

/// lib code exposing `add(a,b)` — A4's test will really assert against it.
const GOOD_CODE: &str = "pub fn add(a: i64, b: i64) -> i64 { a + b }\n";

/// A REAL integration test that fails against the seed but passes against
/// GOOD_CODE — the fairness proof (§5.9 m5): the test genuinely measures.
const GOOD_TEST: &str = "// AC1: add sums two integers\n#[test]\nfn ac1_add_sums() {\n    assert_eq!(pipe_agent::add(2, 3), 5);\n}\n";

fn task(id: &str, agent: &str, deps: &[&str]) -> TaskItem {
    let mut t = TaskItem {
        id: id.into(),
        spec_requirement_ids: vec!["R1".into()],
        title: format!("{agent} work for {id}"),
        description: "pipeline step".into(),
        department: None,
        assigned_agent: agent.into(),
        dependencies: deps.iter().map(|s| s.to_string()).collect(),
        target: "cloud".into(),
        toolchain: Some("rust".into()),
        acceptance: Acceptance {
            verifiable_by: "cargo".into(),
            criteria: "add(2,3) == 5".into(),
        },
        estimated: Some(Estimate {
            tokens_in: 500,
            tokens_out: 200,
            cost_usd: 0.0001,
            duration_seconds: 30,
        }),
        difficulty: None,
        file_ownership: vec![],
        artifacts_expected: vec![],
        artifacts_produced: vec![],
        blocked_by: vec![],
        status: "pending".into(),
        retry_count: 0,
        escalation_level: 0,
        notes: String::new(),
        extra: Default::default(),
    };
    if agent == "A4" {
        // §5.9 m5(a): the plan binds the AC id; A4's files must cite it.
        t.extra
            .insert("acceptance_ids".into(), serde_json::json!(["AC1"]));
    }
    t
}

fn plan() -> TaskPlan {
    TaskPlan {
        schema_version: "1.0".into(),
        spec_id: SPEC.into(),
        created_at: "2026-09-12T00:00:00Z".into(),
        created_by: "A2".into(),
        revision: 1,
        tasks: vec![
            task("T1", "A5", &[]),
            task("T2", "A7", &["T1"]), // §6.3: code → security
            task("T3", "A4", &["T2"]), // §6.3: security → test
        ],
        execution_order: vec!["T1".into(), "T2".into(), "T3".into()],
        parallel_groups: vec![],
        critical_path: vec!["T1".into(), "T2".into(), "T3".into()],
        status_summary: Default::default(),
        extra: Default::default(),
    }
}

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
        run_id: "run-pipe".into(),
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

fn coding(task_id: &str, path: &str, content: &str) -> String {
    serde_json::json!({
        "schema_version": "1.0",
        "task_id": task_id,
        "files": [{ "path": path, "content": content }],
        "self_check": { "commands": ["cargo check"] }
    })
    .to_string()
}

use std::sync::Arc;

struct Bridged {
    inner: AgentBridge,
    calls: RefCell<Vec<(String, String)>>, // (task, agent)
}
impl TaskExecutor for Bridged {
    fn execute(&self, task: &TaskItem, op_id: &str) -> ribozym::Result<Executed> {
        self.calls
            .borrow_mut()
            .push((task.id.clone(), task.assigned_agent.clone()));
        self.inner.execute(task, op_id)
    }
}

fn bridge(workdir: &Path, mock: Arc<MockProvider>) -> AgentBridge {
    AgentBridge::new(
        config::find_repo_root(&std::env::current_dir().unwrap()).unwrap(),
        mock,
        gateway(workdir),
        workdir,
        "ADR: tiny math crate",
    )
}

// -------------------------------------------------------------------- tests

/// §6.3 pipeline end to end: A5 writes the lib, A7 scans it clean, A4 writes
/// a REAL test that REALLY fails against the seed and REALLY passes against
/// the new code — and the run delivers.
#[test]
fn pipeline_a5_a7_a4_delivers_with_real_test_run() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path().join("workdir");
    seed_crate(&workdir);

    // Sanity for the FAIRNESS claim: the test must FAIL on the seed crate
    // (no `add`) and PASS after A5 writes the implementation.
    let g = gateway(&workdir);
    std::fs::create_dir_all(workdir.join("tests")).unwrap();
    std::fs::write(workdir.join("tests").join("ac.rs"), GOOD_TEST).unwrap();
    let failing = g.run("cargo test").unwrap();
    assert_ne!(failing.exit_code, 0, "seed crate must not pass AC1");
    std::fs::remove_file(workdir.join("tests").join("ac.rs")).unwrap();

    let mock = Arc::new(MockProvider::new(vec![
        // T1/A5: implement add()
        MockStep::Success {
            text: coding("T1", "src/lib.rs", GOOD_CODE),
            tokens_in: 500,
            tokens_out: 200,
        },
        // T3/A4: integration test citing AC1 (T2/A7 is deterministic: no LLM)
        MockStep::Success {
            text: coding("T3", "tests/ac1.rs", GOOD_TEST),
            tokens_in: 500,
            tokens_out: 200,
        },
    ]));
    let exec = Bridged {
        inner: bridge(&workdir, mock.clone()),
        calls: RefCell::new(vec![]),
    };
    let mut o = Orchestrator::create(&cfg(dir.path()), &plan()).unwrap();
    assert_eq!(o.advance(&exec, true).unwrap(), Outcome::Delivered);

    // Dispatch order followed the DAG: A5 → A7 → A4.
    assert_eq!(
        *exec.calls.borrow(),
        vec![
            ("T1".to_string(), "A5".to_string()),
            ("T2".to_string(), "A7".to_string()),
            ("T3".to_string(), "A4".to_string()),
        ]
    );
    // LLM used only by A5+A4 (A7 is deterministic).
    assert_eq!(mock.calls.lock().unwrap().len(), 2);
    // The test file really exists under tests/ with the AC citation.
    let test = std::fs::read_to_string(workdir.join("tests").join("ac1.rs")).unwrap();
    assert!(test.contains("AC1"), "§5.9 m5(a) traceability comment");
    // A7's result landed in the ledger as a CLOSED agent op (deterministic,
    // model_usage null — §11.3).
    let store =
        ribozym::state::StateStore::open(&dir.path().join(".ribozym").join("state.db")).unwrap();
    let n: i64 = store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM ledger WHERE state='finished'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 3, "all three agents closed their ops");
}

/// §5.12 m4: a model that embeds a secret in code triggers the CRITICAL
/// path — T2/A7 fails with SECURITY_SECRET_FOUND in the summary, the
/// dependent A4 task blocks, the run terminates failed.
#[test]
fn secret_in_code_is_critical_and_fails_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path().join("workdir");
    seed_crate(&workdir);

    let leaky = "pub const KEY: &str = \"OPENROUTER_API_KEY=sk-proj-abc123\";\n";
    let mock = Arc::new(MockProvider::new(vec![MockStep::Success {
        text: coding("T1", "src/lib.rs", leaky),
        tokens_in: 100,
        tokens_out: 50,
    }]));
    let exec = Bridged {
        inner: bridge(&workdir, mock),
        calls: RefCell::new(vec![]),
    };
    let mut o = Orchestrator::create(&cfg(dir.path()), &plan()).unwrap();
    let out = o.advance(&exec, true).unwrap();
    // §5.12 m4: a CRITICAL security finding parks the run for the human
    // (NeedsUser); §6.1's crisis cap makes blanket "compliant" impossible
    // over red work — the scheduler's truth beats the caller's verdict.
    assert!(matches!(out, Outcome::NeedsUser { .. }), "got {out:?}");
    // Only A5 and A7 ran; A4 never did (blocked by the failed A7).
    assert_eq!(exec.calls.borrow().len(), 2);
    // The critical marker is in the events stream (TaskFailed carries the
    // summary), so the user-facing report can cite it.
    let events =
        std::fs::read_to_string(dir.path().join("runs").join(SPEC).join("events.jsonl")).unwrap();
    assert!(
        events.contains("SECURITY_SECRET_FOUND"),
        "critical security marker must reach the event stream"
    );
}

/// §5.9 m5: a test that cites NO acceptance id is rejected BEFORE writing —
/// the turn fails, and the run terminates with the task failed (the
/// "written to pass" shield is structurally impossible here).
#[test]
fn untraceable_test_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path().join("workdir");
    seed_crate(&workdir);

    let no_citation = "// no AC id anywhere\n#[test]\nfn always() { assert_eq!(1,1); }\n";
    let mock = Arc::new(MockProvider::new(vec![
        MockStep::Success {
            text: coding("T1", "src/lib.rs", GOOD_CODE),
            tokens_in: 100,
            tokens_out: 50,
        },
        MockStep::Success {
            text: coding("T3", "tests/cheap.rs", no_citation),
            tokens_in: 100,
            tokens_out: 50,
        },
    ]));
    let exec = Bridged {
        inner: bridge(&workdir, mock),
        calls: RefCell::new(vec![]),
    };
    let mut o = Orchestrator::create(&cfg(dir.path()), &plan()).unwrap();
    let out = o.advance(&exec, true).unwrap();
    // A4's rejection lands as TaskFailed → crisis cap → NeedsUser (§8.1).
    assert!(matches!(out, Outcome::NeedsUser { .. }), "got {out:?}");
    assert!(
        !workdir.join("tests").join("cheap.rs").exists(),
        "rejected test must not be written"
    );
    let events =
        std::fs::read_to_string(dir.path().join("runs").join(SPEC).join("events.jsonl")).unwrap();
    assert!(events.contains("untraceable test rejected"));
}
