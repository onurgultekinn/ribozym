//! Integration tests — the Rust counterpart of scripts/validate_skeleton.py.
//! Fixture truth lives in examples/expected/ (single source); these tests
//! assert the Rust models round-trip it and the config invariants hold.
//! Test comments per §24 rule 1 style: English.

use std::path::PathBuf;

use ribozym::config;
use ribozym::models::event::EventEnvelope;
use ribozym::models::execution::ExecutionResult;
use ribozym::models::report::DepartmentReport;
use ribozym::models::spec::Spec;
use ribozym::models::task::TaskPlan;
use ribozym::validate;

fn repo_root() -> PathBuf {
    config::find_repo_root(&std::env::current_dir().unwrap())
        .or_else(|| config::find_repo_root(&PathBuf::from(env!("CARGO_MANIFEST_DIR"))))
        .expect("repo root with ribozym.toml")
}

fn fixture(name: &str) -> String {
    let p = repo_root().join("examples").join("expected").join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

/// Option-typed fields serialize None as absent while fixtures may carry
/// explicit `null`; for schema purposes null == absent (§11.0). Normalize both
/// sides by dropping null-valued keys before comparing.
fn normalize(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            map.retain(|_, val| !val.is_null());
            map.values_mut().for_each(normalize);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(normalize),
        _ => {}
    }
}

fn assert_round_trip(fixture_name: &str, back: serde_json::Value) {
    let mut orig: serde_json::Value = serde_json::from_str(&fixture(fixture_name)).unwrap();
    let mut back = back;
    normalize(&mut orig);
    normalize(&mut back);
    assert_eq!(back, orig, "round-trip must be lossless for {fixture_name}");
}

#[test]
fn spec_fixture_round_trips() {
    let doc: Spec = serde_json::from_str(&fixture("spec_valid.json")).unwrap();
    // ADR mandatory fields survived (v2.6)
    assert_eq!(doc.decisions.len(), 1);
    assert!(!doc.decisions[0].rationale.is_empty());
    assert_eq!(doc.decisions[0].alternatives_considered.len(), 2);
    assert_round_trip("spec_valid.json", serde_json::to_value(&doc).unwrap());
}

#[test]
fn tasks_fixture_round_trips() {
    let doc: TaskPlan = serde_json::from_str(&fixture("tasks_valid.json")).unwrap();
    assert_eq!(doc.tasks.len(), 2);
    // Every task links >= 1 requirement (§6.4 traceability rule)
    assert!(doc.tasks.iter().all(|t| !t.spec_requirement_ids.is_empty()));
    // T2 estimate is null -> "no data", never invented (v2.24)
    assert!(doc.tasks[1].estimated.is_none());
    // Difficulty path recorded (v2.44)
    assert_eq!(doc.tasks[1].difficulty.as_ref().unwrap().source, "llm");
    assert_round_trip("tasks_valid.json", serde_json::to_value(&doc).unwrap());
}

#[test]
fn execution_result_fixture_round_trips() {
    let doc: ExecutionResult =
        serde_json::from_str(&fixture("execution_result_valid.json")).unwrap();
    assert!(doc.model_usage.is_some());
    assert_eq!(doc.idempotency.as_ref().unwrap().op_id, "op-77b1");
    assert_round_trip(
        "execution_result_valid.json",
        serde_json::to_value(&doc).unwrap(),
    );
}

#[test]
fn report_fixture_round_trips() {
    let doc: DepartmentReport = serde_json::from_str(&fixture("report_valid.json")).unwrap();
    assert_eq!(doc.generated_by, "A3");
    // P5: Delivered only at spec_match_pct == 100
    assert_eq!(doc.compliance.spec_match_pct, 100);
    // Traceability map present (§11.5)
    assert_eq!(doc.traceability.len(), 2);
    assert_round_trip("report_valid.json", serde_json::to_value(&doc).unwrap());
}

#[test]
fn events_fixture_parses_and_seq_increases() {
    let raw = fixture("events_valid.jsonl");
    let mut last_seq = 0;
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let ev: EventEnvelope = serde_json::from_str(line).unwrap();
        assert!(ev.seq > last_seq, "seq must strictly increase (§7.3)");
        last_seq = ev.seq;
    }
    assert!(last_seq >= 5);
}

#[test]
fn unknown_event_kind_is_preserved_as_unknown() {
    // Forward-compat: a future event name must not break resume (§11.0).
    let line = r#"{"seq":9,"ts":"2026-09-12T10:00:00Z","spec_id":"spec-20260912-001","run_id":"run-x1","event":"FutureThing","correlation_id":"op-1","schema_version":"1.0"}"#;
    let ev: EventEnvelope = serde_json::from_str(line).unwrap();
    assert!(matches!(
        ev.event,
        ribozym::models::event::EventKind::Unknown
    ));
}

#[test]
fn agents_registry_is_contiguous_and_prompted() {
    let root = repo_root();
    let agents = config::load_agents(&root).unwrap();
    let ids: Vec<&str> = agents.iter().map(|a| a.id.as_str()).collect();
    let expected: Vec<String> = (1..=12).map(|i| format!("A{i}")).collect();
    let expected: Vec<&str> = expected.iter().map(|s| s.as_str()).collect();
    assert_eq!(ids, expected);
    let share: u32 = agents.iter().map(|a| a.budget_share_pct).sum();
    assert_eq!(share, 100);
    for a in &agents {
        assert!(
            root.join(&a.prompt_file).is_file(),
            "missing prompt for {}: {}",
            a.id,
            a.prompt_file
        );
    }
}

#[test]
fn config_validate_passes_on_this_repo() {
    let root = repo_root();
    let problems = validate::validate_config(&root).unwrap();
    assert!(problems.is_empty(), "config problems: {problems:?}");
}

#[test]
fn local_grunt_list_wires_to_router_and_excludes_core_tasks() {
    let root = repo_root();
    let models = config::load_models(&root).unwrap();
    let eligible = &models.local.eligible_task_types;
    assert!(
        !eligible.is_empty(),
        "§10.1: local must have an explicit grunt list"
    );
    // The Q1 core lanes must NEVER be local-eligible.
    for core in ["plan", "code", "verify", "report"] {
        assert!(
            !eligible.iter().any(|t| t == core),
            "core task '{core}' must not be in [local].eligible_task_types"
        );
    }
    // Router classification is pure config membership (P3: no LLM decides).
    assert_eq!(
        ribozym::select::classify("boilerplate", eligible),
        ribozym::select::WorkClass::Grunt
    );
    assert_eq!(
        ribozym::select::classify("code", eligible),
        ribozym::select::WorkClass::Core
    );
    // Q1 model identity is the local coder, not a cloud model.
    assert!(models.local.model_id.contains("qwen2.5-coder-7b"));
}

#[test]
fn config_validate_flags_broken_registry() {
    // Build a temp repo with one agent missing its prompt -> must be flagged.
    let td = tempfile_dir();
    std::fs::create_dir_all(td.join("config")).unwrap();
    std::fs::write(
        td.join("ribozym.toml"),
        "[errors]\nfinal_review_crisis_max = 2\n",
    )
    .unwrap();
    std::fs::write(
        td.join("config").join("agents.toml"),
        r#"[[agent]]
id = "A1"
title = "X"
prompt_file = "config/prompts/missing.md"
model_tier = "high"
escalation_tier = "high"
"#,
    )
    .unwrap();
    std::fs::write(td.join("config").join("models.toml"), "").unwrap();
    let problems = validate::validate_config(&td).unwrap();
    assert!(problems.iter().any(|p| p.contains("prompt_file missing")));
    assert!(problems.iter().any(|p| p.contains("budget_share_pct")));
    std::fs::remove_dir_all(td).ok();
}

#[test]
fn schema_validate_rejects_missing_required_fields() {
    let doc = serde_json::json!({"schema_version": "1.0"}); // event missing seq etc.
    let problems = validate::validate_document("event.schema.json", &doc);
    assert!(problems.iter().any(|p| p.contains("'seq'")));
    assert!(problems.iter().any(|p| p.contains("'correlation_id'")));
}

#[test]
fn schema_validate_rejects_bad_spec_id_pattern() {
    let doc = serde_json::json!({
        "seq": 1, "ts": "2026-09-12T10:00:00Z",
        "spec_id": "spec-1", "run_id": "run-x1",
        "event": "TaskStarted", "correlation_id": "op-1", "schema_version": "1.0"
    });
    let problems = validate::validate_document("event.schema.json", &doc);
    assert!(problems.iter().any(|p| p.contains("spec_id")));
}

#[test]
fn migration_creates_all_tables_and_enforces_pk() {
    let db = tempfile_dir().join("state.db");
    let store = ribozym::state::StateStore::open(&db).unwrap();
    let have: Vec<String> = store
        .conn()
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    for t in [
        "runs",
        "tasks",
        "ledger",
        "model_usage",
        "task_scores",
        "approvals",
        "web_cache",
        "prices",
        "archive_sections",
    ] {
        assert!(have.iter().any(|h| h == t), "missing table {t}");
    }
    // PK duplicate rejection
    store
        .conn()
        .execute("INSERT INTO tasks(spec_id, task_id, status, agent) VALUES ('spec-20260912-001','T1','pending','A5')", [])
        .unwrap();
    let dup = store.conn().execute(
        "INSERT INTO tasks(spec_id, task_id, status, agent) VALUES ('spec-20260912-001','T1','done','A5')",
        [],
    );
    assert!(dup.is_err(), "duplicate task PK must be rejected");
}

fn tempfile_dir() -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "ribozym-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&base).unwrap();
    base
}
