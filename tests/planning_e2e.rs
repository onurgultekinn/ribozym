//! Faz 5-c/5-d end-to-end: the FULL management line through the REAL §6.1
//! lifecycle — a user form (saved as the run's provenance file), A1's draft,
//! the spec gate, A2's plan, execution (A5→A7→A4 through the real gateway),
//! A3's report with numbers only CODE wrote, and A1's sealed verdict. LLM is
//! scripted (§21.1); every other layer — schema guards, traceability, gates,
//! ledger, budget, real `cargo` — is the production one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use ribozym::agents::AgentBridge;
use ribozym::config;
use ribozym::gateway::ToolGateway;
use ribozym::hash;
use ribozym::inference::{MockProvider, MockStep};
use ribozym::orchestrator::{Orchestrator, Outcome, PlanningOutcome, RunConfig};
use ribozym::pipeline::AgentPipeline;
use ribozym::repair::SchemaGuard;
use ribozym::state::StateStore;
use ribozym::validate;

const SPEC: &str = "spec-20260912-777";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn seed_crate(dir: &Path) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"plan-agent\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("src").join("lib.rs"), "// seed\n").unwrap();
}

const GOOD_CODE: &str = "pub fn add(a: i64, b: i64) -> i64 { a + b }\n";
const GOOD_TEST: &str = "// AC1: add sums two integers\n#[test]\nfn ac1_add_sums() {\n    assert_eq!(plan_agent::add(2, 3), 5);\n}\n";

/// A filled form that passes the §5.4 structural check.
const FORM: &[u8] = b"# Spec Form: tiny adder\n\n## Summary\nadd two numbers, prove it\n\n## Goal & Users\n- goal: demo\n\n## Scope\n- must_have:\n  - add(a, b)\n\n## Constraints\n- rust only\n\n## Budget\n- run_budget_usd: 1.5\n\n## Delivery Shape\ncrate\n\n## Success Criteria\ncargo test passes\n";

/// A1's scripted answer to the form. The hash it claims is garbage on
/// purpose — code must overwrite it with the real form bytes (§5.4C).
fn spec_draft() -> Value {
    json!({
        "schema_version": "1.0",
        "id": SPEC,
        "version": "1.0.0",
        "created_at": "2026-09-12T09:00:00Z",
        "created_by": "A1",
        "source": {
            "raw_input_hash": "sha256:deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
            "raw_input_path": "claimed/elsewhere.md"
        },
        "title": "Tiny adder",
        "description": "A bounded crate computing add(2,3) == 5.",
        "mode": "greenfield",
        "language": { "primary": "rust", "confidence": 0.95, "source": "explicit" },
        "requirements": [
            { "id": "R1", "text": "add(a,b) sums two integers", "priority": "must", "category": "functional" }
        ],
        "constraints": [],
        "acceptance_criteria": [
            { "id": "AC1", "text": "cargo test passes the integration test", "verifiable_by": "A4", "measurement": "exit_code == 0" }
        ],
        "out_of_scope": ["CLI"],
        "assumptions": [{ "id": "AS1", "assumption": "integer inputs only" }],
        "decisions": [{
            "id": "D1", "title": "language", "choice": "rust",
            "alternatives_considered": [{ "option": "python", "rejected_because": "toolchain policy" }],
            "rationale": "explicit in form", "evidence": ["form: Constraints"]
        }],
        "budget": { "max_cost_usd": 1.50, "max_duration_seconds": 3600 },
        "autonomy_level": 3,
        "approval_required": ["delivery"],
        "compliance_status": "approved"
    })
}

/// A2's scripted answer: the §6.3 code lane, one chain, honest nulls.
fn task_plan() -> Value {
    let task = |id: &str, agent: &str, deps: &[&str]| {
        json!({
            "id": id,
            "spec_requirement_ids": ["R1"],
            "title": format!("{agent} step {id}"),
            "description": "code lane",
            "assigned_agent": agent,
            "dependencies": deps,
            "target": "cloud",
            "toolchain": "rust",
            "acceptance": { "verifiable_by": "cargo", "criteria": "add(2,3) == 5" },
            "estimated": null,
            "status": "pending",
            "acceptance_ids": ["AC1"]
        })
    };
    json!({
        "schema_version": "1.0",
        "spec_id": SPEC,
        "created_at": "2026-09-12T09:05:00Z",
        "created_by": "A2",
        "revision": 1,
        "tasks": [task("T1", "A5", &[]), task("T2", "A7", &["T1"]), task("T3", "A4", &["T2"])],
        "status_summary": { "total": 3, "pending": 3, "in_progress": 0, "done": 0, "failed": 0, "skipped": 0 }
    })
}

fn coding(task_id: &str, path: &str, content: &str) -> Value {
    json!({
        "schema_version": "1.0",
        "task_id": task_id,
        "files": [{ "path": path, "content": content }],
        "self_check": { "commands": if task_id == "T1" { vec!["cargo check"] } else { vec!["cargo test"] } }
    })
}

/// A3's scripted narrative — with LIES in the numeric fields. Code keeps
/// only the story (§5.7 m2).
fn narrative_reply() -> Value {
    json!({
        "schema_version": "1.0",
        "spec_id": SPEC,
        "generated_at": "2026-09-12T12:00:00Z",
        "generated_by": "A3",
        "final_status": "success",
        "summary": { "tasks_total": 99, "tasks_done": 99, "tasks_failed": 0, "tasks_skipped": 0, "retries": 0, "escalations": 0 },
        "cost": { "total_usd": 999.0, "breakdown": {}, "cache_savings_usd": 0.0, "budget_limit_usd": 1.0, "budget_used_pct": 99900.0 },
        "executive_summary": "All three lanes delivered: code written, scanned, and genuinely tested."
    })
}

fn compliance_reply(status: &str) -> Value {
    json!({
        "schema_version": "1.0",
        "spec_id": SPEC,
        "checked_by": "A1",
        "spec_match_pct": 100,
        "requirements": [
            { "id": "R1", "status": status, "evidence": "tests/ac1.rs asserts add(2,3)==5", "test_id": "AC1" }
        ],
        "assumptions_audit": [{ "assumption_id": "AS1", "held": true }],
        "verdict": "approved"
    })
}

fn cfg(root: &Path, crisis_max: u32) -> RunConfig {
    RunConfig {
        root: root.to_path_buf(),
        spec_id: SPEC.into(),
        run_id: "run-plan".into(),
        autonomy_level: 3,
        batch_yes: true,
        max_concurrency: 1,
        crisis_max,
        gate_timeout_seconds: 1200,
        budget_usd: 5.0,
        budget_soft_pct: 80.0,
        budget_hard_pct: 100.0,
        now_epoch: 1_789_207_200,
    }
}

/// Everything scripted in order for a run where the referee tells the truth.
fn honest_script() -> Vec<MockStep> {
    [
        spec_draft(),
        task_plan(),
        coding("T1", "src/lib.rs", GOOD_CODE),
        coding("T3", "tests/ac1.rs", GOOD_TEST),
        narrative_reply(),
        compliance_reply("met"),
    ]
    .into_iter()
    .map(|v| MockStep::Success {
        text: v.to_string(),
        tokens_in: 300,
        tokens_out: 120,
    })
    .collect()
}

fn gateway(workdir: &Path) -> Arc<ToolGateway> {
    let project = config::load_project_config(&repo_root()).unwrap();
    Arc::new(ToolGateway::new(
        project.toolchains,
        workdir.to_path_buf(),
        Arc::new(ribozym::gateway::SystemRunner),
        std::env::vars().collect(),
    ))
}

/// Run the WHOLE line once in `dir` — the real §6.1 lifecycle from the form
/// (idle) through the spec gate to Scheduling, then execution to the outcome.
fn run_pipeline(dir: &Path, steps: Vec<MockStep>) -> (Outcome, String) {
    let workdir = dir.join("workdir");
    seed_crate(&workdir);
    let mock = Arc::new(MockProvider::new(steps));
    let guard = Arc::new(SchemaGuard::new(mock.clone(), repo_root()));
    let c = cfg(dir, 2);

    // Step 1: the management line (A1 draft → spec gate → A2 plan).
    let mut o = Orchestrator::create_idle(&c).unwrap();
    let mut planning =
        AgentPipeline::for_form(c.clone(), repo_root(), guard.clone(), FORM.to_vec());
    assert_eq!(
        o.advance_planning(&mut planning).unwrap(),
        PlanningOutcome::Planned,
        "autonomy 3 + batch: the spec gate passes without waiting"
    );
    // Provenance: the saved spec names the REAL saved form bytes.
    let spec_doc: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("runs").join(SPEC).join("spec.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        spec_doc["source"]["raw_input_hash"],
        format!("sha256:{}", hash::sha256_hex(FORM)),
        "provenance overwritten with the real hash of the real bytes"
    );
    assert!(
        dir.join("runs").join(SPEC).join("raw_input.md").is_file(),
        "the form itself is preserved as the provenance file"
    );
    // The spec's budget replaced the config default at the run row.
    let store = StateStore::open(&dir.join(".ribozym").join("state.db")).unwrap();
    let b: f64 = store
        .conn()
        .query_row("SELECT budget_usd FROM runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(b, 1.5, "the validated spec owns the run budget (§8.2)");
    let plan = planning.plan().expect("the plan document persisted");
    assert_eq!(plan.tasks.len(), 3);

    // Step 2: the run itself — bridge executes the DAG, pipeline reports.
    let bridge = AgentBridge::new(
        repo_root(),
        mock.clone(),
        gateway(&workdir),
        &workdir,
        "ADR: tiny math crate",
    );
    let mut pipeline = AgentPipeline::for_resume(c, repo_root(), guard);
    let outcome = o.advance_with(&bridge, &mut pipeline).unwrap();
    (outcome, o.final_digest().unwrap())
}

#[test]
fn form_to_delivered_pipeline_reports_sealed_truth() {
    let dir = tempfile::tempdir().unwrap();
    let (outcome, _hash) = run_pipeline(dir.path(), honest_script());
    assert_eq!(outcome, Outcome::Delivered, "{outcome:?}");

    let rundir = dir.path().join("runs").join(SPEC);
    // The sealed report exists and is schema-valid.
    let raw = std::fs::read_to_string(rundir.join("report.json")).unwrap();
    let doc: Value = serde_json::from_str(&raw).unwrap();
    assert!(
        validate::validate_full(&repo_root(), "department_report.schema.json", &doc).is_empty(),
        "sealed report must pass its own schema"
    );
    // Numbers are CODE's, not the narrative model's lies: 3 tasks, mock $0.
    assert_eq!(doc["summary"]["tasks_total"], 3);
    assert_eq!(doc["summary"]["tasks_done"], 3);
    assert_eq!(doc["cost"]["total_usd"], 0.0);
    assert_eq!(doc["final_status"], "success");
    // The narrative survived the merge — the story came from the model.
    assert!(doc["executive_summary"]
        .as_str()
        .unwrap()
        .contains("genuinely tested"));
    // The sealed compliance block cites the referee's evidence.
    assert_eq!(doc["compliance"]["spec_match_pct"], 100);
    assert_eq!(doc["compliance"]["checked_by"], "A1");
    assert_eq!(doc["compliance"]["acceptance_criteria_passed"][0], "AC1");
    // A1's raw verdict is separately auditable, and every planning/execution
    // document the lifecycle produced is on disk.
    assert!(rundir.join("compliance.json").is_file());
    assert!(rundir.join("spec.json").is_file());
    assert!(rundir.join("plan.json").is_file());
    assert!(rundir.join("raw_input.md").is_file());

    // The event stream tells the WHOLE §6.1 story: form → spec → plan →
    // delivery, in order.
    let events = std::fs::read_to_string(rundir.join("events.jsonl")).unwrap();
    let seq_of = |name: &str| -> u64 {
        events
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .find(|v| v["event"] == name)
            .unwrap_or_else(|| panic!("event {name} missing"))["seq"]
            .as_u64()
            .unwrap()
    };
    let (sr, sv, tc, dc) = (
        seq_of("SpecReceived"),
        seq_of("SpecValidated"),
        seq_of("TasksCreated"),
        seq_of("Delivered"),
    );
    assert!(
        sr < sv && sv < tc && tc < dc,
        "lifecycle order broken: {sr} {sv} {tc} {dc}"
    );

    // The report's traceability map agrees with §6.4: R1 → all three tasks.
    let trace = &doc["traceability"][0];
    assert_eq!(trace["requirement_id"], "R1");
    assert_eq!(trace["status"], "met");
    for id in ["T1", "T2", "T3"] {
        assert!(
            trace["task_ids"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t == id),
            "every task citing R1 shows up"
        );
    }
    // The per-task records carry the model-usage truth the bridge wrote.
    let t1 = doc["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "T1")
        .unwrap();
    assert_eq!(t1["tokens"]["in"], 300);
    assert_eq!(t1["model"], "mock/mock-1");
    assert_eq!(
        t1["cost_usd"], 0.0,
        "mock bills honestly zero, not silently"
    );

    // The ledger proves planning turns were guarded too: four model calls
    // (draft, plan, narrate, review) charged exactly once each.
    let store = StateStore::open(&dir.path().join(".ribozym").join("state.db")).unwrap();
    let (plan_ops, charged): (i64, i64) = store
        .conn()
        .query_row(
            "SELECT COUNT(*), SUM(charged_at IS NOT NULL) FROM ledger WHERE kind='plan_turn'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(plan_ops, 4);
    assert_eq!(charged, plan_ops);
}

#[test]
fn read_only_surfaces_project_the_sealed_truth() {
    // status/report/budget read the RECORD — the same run the pipeline
    // delivered must surface identically through the public API (P1: the
    // CLI is a thin wrapper, and the surfaces never recompute).
    let dir = tempfile::tempdir().unwrap();
    let (outcome, _) = run_pipeline(dir.path(), honest_script());
    assert_eq!(outcome, Outcome::Delivered);

    let runs = ribozym::surface::status_all(dir.path()).unwrap();
    assert_eq!(runs.len(), 1);
    let r = &runs[0];
    assert_eq!(r.spec_id, SPEC);
    assert_eq!(r.phase, "delivered");
    assert_eq!(r.tasks.get("done"), Some(&3));
    assert_eq!(r.budget_usd, 1.5, "the spec owns the frame");
    assert_eq!(r.spent_usd, 0.0, "mock bills zero honestly");
    assert_eq!(r.pending_gate, None);

    let doc = ribozym::surface::report_json(dir.path(), SPEC).unwrap();
    assert_eq!(doc["summary"]["tasks_done"], 3);
    let md = ribozym::surface::render_md(&doc);
    assert!(md.contains(&format!("# Run report — {SPEC}")));
    assert!(md.contains("spec_match: **100%**"));
    assert!(md.contains("genuinely tested"));
    assert!(md.contains("R1"), "traceability renders");

    // A spec that never ran has no report — and the surface says so loudly.
    let err = ribozym::surface::report_json(dir.path(), "spec-20260912-001").unwrap_err();
    assert!(err.to_string().contains("no sealed report"), "{err}");

    let b = ribozym::surface::budget_surface(dir.path(), 5.0, 80.0, 100.0).unwrap();
    assert_eq!(b.runs.len(), 1);
    assert_eq!(b.total_spent, 0.0);
}

#[test]
fn archive_tampering_is_caught_on_read_never_silently_ignored() {
    // §7.6 B2: after a delivered run, mutate the primary spec document.
    // The next lifecycle read must scream ARCHIVE_CORRUPT — silent
    // continue would let the tampered bytes feed a later turn.
    let dir = tempfile::tempdir().unwrap();
    let (outcome, _) = run_pipeline(dir.path(), honest_script());
    assert_eq!(outcome, Outcome::Delivered);
    let spec_path = dir.path().join("runs").join(SPEC).join("spec.json");
    let mut doc: Value =
        serde_json::from_str(&std::fs::read_to_string(&spec_path).unwrap()).unwrap();
    doc["title"] = json!("quietly rewritten");
    std::fs::write(&spec_path, serde_json::to_vec(&doc).unwrap()).unwrap();

    let guard = Arc::new(SchemaGuard::new(
        Arc::new(MockProvider::new(vec![])),
        repo_root(),
    ));
    let pipeline = AgentPipeline::for_resume(cfg(dir.path(), 2), repo_root(), guard);
    let err = pipeline.spec().unwrap_err();
    assert!(
        err.to_string().contains("ARCHIVE_CORRUPT"),
        "tamper must be fatal on read, got: {err}"
    );
    // The archive BACKUP itself is intact — the sweep proves the two
    // concerns are separate: backup integrity (status) vs read-path
    // verification (above). Tampering the live document never silently
    // repairs the archive; repair_section reseals FROM the source.
    let runs = ribozym::surface::status_all(dir.path()).unwrap();
    assert!(runs[0].archive_corrupt.is_empty());
}

#[test]
fn two_identical_runs_produce_identical_final_digests() {
    // §21.2: determinism across whole fresh pipelines (planning included).
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let (_, ha) = run_pipeline(a.path(), honest_script());
    let (_, hb) = run_pipeline(b.path(), honest_script());
    assert_eq!(ha, hb, "same scripted world must converge to one hash");
}

#[test]
fn lying_referee_cannot_deliver_and_parks_finitely() {
    // The model claims 100/approved while marking R1 UNMET. adjudicate
    // recomputes the score from the statuses → 0 → no delivery. crisis_max
    // caps the replan loop: NeedsUser, not an eternal Crisis spin. The
    // crisis re-review REUSES the saved verdict (rule 4 over the identical
    // evidence), so ONE lying step is consumed even though the gate is
    // visited three times.
    let dir = tempfile::tempdir().unwrap();
    let mut steps = honest_script();
    steps.remove(5); // drop the honest verdict
    let cc = MockStep::Success {
        text: compliance_reply("unmet").to_string(),
        tokens_in: 300,
        tokens_out: 120,
    };
    let same = MockStep::Success {
        text: task_plan().to_string(),
        tokens_in: 300,
        tokens_out: 120,
    };
    // The E2 hook now ACTS: each crisis invites a replan turn. The revised
    // plan cannot add in-scope work for a lie ("R1 unmet" while every task
    // and test passed), so the CEO returns the same cut, the crisis cap
    // escalates — and the reviewer was invoiced ONCE (rule 4 over the
    // identical evidence hash).
    steps.extend([cc, same.clone(), same]);
    let (outcome, _) = run_pipeline(dir.path(), steps);
    assert!(
        matches!(outcome, Outcome::NeedsUser { .. }),
        "got {outcome:?}"
    );
    // The sealed report shows the CODE-recomputed 0%, never the claimed 100.
    let raw =
        std::fs::read_to_string(dir.path().join("runs").join(SPEC).join("report.json")).unwrap();
    let doc: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(doc["compliance"]["spec_match_pct"], 0);
    assert_eq!(doc["compliance"]["unmet_requirements"][0], "R1");
    // ...while the work itself IS done — the report tells both truths at once.
    assert_eq!(doc["summary"]["tasks_done"], 3);
    // Rule-4 recovery proof: the reviewer model was invoiced ONCE.
    let store = StateStore::open(&dir.path().join(".ribozym").join("state.db")).unwrap();
    let reviews: i64 = store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM ledger WHERE kind='plan_turn' AND task_id='a1-review'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reviews, 1, "a crisis re-review never re-calls the model");
}

#[test]
fn an_incomplete_form_never_reaches_the_model() {
    // §6.1 FormInvalidExhausted: the structural check is CODE's. The mock
    // script is EMPTY — reaching any LLM turn would error the run.
    let dir = tempfile::tempdir().unwrap();
    let mock = Arc::new(MockProvider::new(vec![]));
    let guard = Arc::new(SchemaGuard::new(mock.clone(), repo_root()));
    let c = cfg(dir.path(), 2);
    let mut o = Orchestrator::create_idle(&c).unwrap();
    let mut planning = AgentPipeline::for_form(
        c,
        repo_root(),
        guard,
        b"# Spec Form: nothing\n\n## Summary\n<!-- TODO -->\n\n## Scope\n- must_have: []\n\n## Budget\n- run_budget_usd: 0\n\n## Delivery Shape\n\n## Success Criteria\n\n## Constraints\n".to_vec(),
    );
    match o.advance_planning(&mut planning).unwrap() {
        PlanningOutcome::NeedsUser { why } => {
            assert!(why.contains("must_have"), "{why}");
        }
        other => panic!("expected NeedsUser, got {other:?}"),
    }
    assert!(
        mock.calls.lock().unwrap().is_empty(),
        "no turn after a failed form check"
    );
}

#[test]
fn rest_api_form_golden_plan_is_valid_and_traceable() {
    // The §22 golden input end to end at the PLANNING level: the real
    // examples/spec_rest_api.md through the full management line (A1 draft +
    // spec gate + A2 plan). Execution needs `axum`, outside the offline
    // toolchain — the machinery of the 3-requirement graph is what pins.
    let dir = tempfile::tempdir().unwrap();
    let form = std::fs::read(repo_root().join("examples/spec_rest_api.md")).unwrap();
    let mock = Arc::new(MockProvider::new(vec![
        MockStep::Success {
            text: rest_api_spec().to_string(),
            tokens_in: 900,
            tokens_out: 400,
        },
        MockStep::Success {
            text: rest_api_plan().to_string(),
            tokens_in: 900,
            tokens_out: 400,
        },
    ]));
    let guard = Arc::new(SchemaGuard::new(mock, repo_root()));
    let c = cfg(dir.path(), 2);
    let mut o = Orchestrator::create_idle(&c).unwrap();
    let mut planning = AgentPipeline::for_form(c, repo_root(), guard, form.clone());
    assert_eq!(
        o.advance_planning(&mut planning).unwrap(),
        PlanningOutcome::Planned
    );
    let spec = planning.spec().unwrap();
    let plan = planning.plan().unwrap();

    assert_eq!(
        spec.source.raw_input_hash,
        format!("sha256:{}", hash::sha256_hex(&form))
    );
    assert!(validate::validate_full(
        &repo_root(),
        "spec.schema.json",
        &serde_json::to_value(&spec).unwrap()
    )
    .is_empty());
    assert!(validate::check_traceability(&spec, &plan).is_empty());
    assert_eq!(spec.requirements.len(), 3);
    // The three MUSTs each carry at least one task; execution order is code-
    // derived topological truth, not the model's suggestion.
    assert_eq!(
        plan.execution_order,
        vec![
            "T1".to_string(),
            "T2".to_string(),
            "T3".to_string(),
            "T4".to_string()
        ]
    );
    assert!(plan.tasks.iter().all(|t| t.estimated.is_none()));
}

#[test]
fn crisis_replan_recovers_and_delivers_on_the_second_verdict() {
    // E2 end to end: the plan covered R2 on paper, the referee found it
    // UNMET in the evidence → Crisis → A2 replans IN SCOPE (T1..T3 kept
    // verbatim, recovery task T4 added) → T4 runs → the re-review tells
    // the truth → sealed at 100 → Delivered.
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path().join("workdir");
    seed_crate(&workdir);
    let steps = [
        spec_two_reqs(),
        plan_two_reqs(),
        coding("T1", "src/lib.rs", GOOD_CODE),
        coding("T3", "tests/ac1.rs", GOOD_TEST),
        narrative_reply(),
        compliance_reply_gap(),
        plan_two_reqs_revised(),
        coding(
            "T4",
            "src/versioned.rs",
            "pub const VERSION: &str = \"1\";\n",
        ),
        narrative_reply(),
        compliance_reply_full(),
    ]
    .into_iter()
    .map(|v| MockStep::Success {
        text: v.to_string(),
        tokens_in: 300,
        tokens_out: 120,
    })
    .collect();
    let mock = Arc::new(MockProvider::new(steps));
    let guard = Arc::new(SchemaGuard::new(mock.clone(), repo_root()));
    let c = cfg(dir.path(), 2);
    let mut o = Orchestrator::create_idle(&c).unwrap();
    let mut planning =
        AgentPipeline::for_form(c.clone(), repo_root(), guard.clone(), FORM.to_vec());
    assert_eq!(
        o.advance_planning(&mut planning).unwrap(),
        PlanningOutcome::Planned
    );
    let bridge = AgentBridge::new(
        repo_root(),
        mock.clone(),
        gateway(&workdir),
        &workdir,
        "ADR: tiny math crate",
    );
    let mut pipeline = AgentPipeline::for_resume(c, repo_root(), guard);
    let out = o.advance_with(&bridge, &mut pipeline).unwrap();
    assert_eq!(out, Outcome::Delivered, "{out:?}");

    // The revised plan is on disk at revision 2, completed rows mirrored.
    let plan = pipeline.plan().unwrap();
    assert_eq!(plan.revision, 2, "revision is code-stamped old+1");
    assert_eq!(plan.tasks.len(), 4);
    // The revised document mirrors the DB truth AT REPLAN TIME: the three
    // completed tasks are `done`, the recovery task `T4` was still pending.
    let doc_done: Vec<String> = plan
        .tasks
        .iter()
        .filter(|t| t.status == "done")
        .map(|t| t.id.clone())
        .collect();
    assert_eq!(
        doc_done,
        vec!["T1".to_string(), "T2".to_string(), "T3".to_string()]
    );
    let t4 = plan.tasks.iter().find(|t| t.id == "T4").unwrap();
    assert_eq!(t4.status, "pending", "honest intent, not a pre-written win");
    // Final execution truth lives in the tasks table:
    let store = StateStore::open(&dir.path().join(".ribozym").join("state.db")).unwrap();
    let t4_status: String = store
        .conn()
        .query_row("SELECT status FROM tasks WHERE task_id='T4'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(t4_status, "done");

    // The sealed report cites the RECOVERY task under R2's trace.
    let doc: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("runs").join(SPEC).join("report.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(doc["compliance"]["spec_match_pct"], 100);
    let r2 = doc["traceability"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["requirement_id"] == "R2")
        .unwrap();
    assert!(r2["task_ids"].as_array().unwrap().iter().any(|t| t == "T4"));
    assert_eq!(doc["summary"]["tasks_total"], 4);

    // The archive kept EVERY sealed revision — history is append-only.
    let a = ribozym::archive::Archive::open(dir.path(), SPEC);
    let manifest = a.manifest().unwrap();
    let plan_seals: Vec<&str> = manifest
        .sections
        .iter()
        .filter(|x| x.name.ends_with("plan.json"))
        .map(|x| x.name.as_str())
        .collect();
    assert_eq!(
        plan_seals.len(),
        2,
        "original + revision sealed: {plan_seals:?}"
    );
    assert!(a.verify_all().unwrap().is_empty());
    // The crisis counter survived exactly one revision loop.
    let store = StateStore::open(&dir.path().join(".ribozym").join("state.db")).unwrap();
    let crisis: i64 = store
        .conn()
        .query_row("SELECT crisis_count FROM runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(crisis, 1);
}

/// R1+R2 spec (the gap scenario's locked contract).
fn spec_two_reqs() -> Value {
    let mut v = spec_draft();
    v["requirements"] = json!([
        { "id": "R1", "text": "add(a,b) sums two integers", "priority": "must", "category": "functional" },
        { "id": "R2", "text": "a versioned constant exists", "priority": "must", "category": "functional" }
    ]);
    v
}

fn base_two_tasks() -> Vec<Value> {
    vec![
        json!({
            "id": "T1", "spec_requirement_ids": ["R1", "R2"], "title": "A5 core",
            "description": "code lane", "assigned_agent": "A5", "dependencies": [],
            "target": "cloud", "toolchain": "rust",
            "acceptance": { "verifiable_by": "cargo", "criteria": "add(2,3) == 5" },
            "estimated": null, "status": "pending"
        }),
        json!({
            "id": "T2", "spec_requirement_ids": ["R1"], "title": "A7 scan",
            "description": "security", "assigned_agent": "A7", "dependencies": ["T1"],
            "target": "cloud", "toolchain": "rust",
            "acceptance": { "verifiable_by": "A7", "criteria": "clean" },
            "estimated": null, "status": "pending"
        }),
        json!({
            "id": "T3", "spec_requirement_ids": ["R1"], "title": "A4 tests",
            "description": "tests", "assigned_agent": "A4", "dependencies": ["T2"],
            "target": "cloud", "toolchain": "rust",
            "acceptance": { "verifiable_by": "cargo", "criteria": "cargo test" },
            "estimated": null, "status": "pending", "acceptance_ids": ["AC1"]
        }),
    ]
}

fn plan_two_reqs() -> Value {
    json!({
        "schema_version": "1.0", "spec_id": SPEC, "created_at": "2026-09-12T09:05:00Z",
        "created_by": "A2", "revision": 1, "tasks": base_two_tasks(),
        "status_summary": { "total": 3, "pending": 3, "in_progress": 0, "done": 0, "failed": 0, "skipped": 0 }
    })
}

fn plan_two_reqs_revised() -> Value {
    let mut tasks = base_two_tasks();
    tasks.push(json!({
        "id": "T4", "spec_requirement_ids": ["R2"], "title": "recovery: versioned constant",
        "description": "covers the unmet R2", "assigned_agent": "A5", "dependencies": ["T3"],
        "target": "cloud", "toolchain": "rust",
        "acceptance": { "verifiable_by": "cargo", "criteria": "cargo check" },
        "estimated": null, "status": "pending"
    }));
    json!({
        "schema_version": "1.0", "spec_id": SPEC, "created_at": "2026-09-12T09:40:00Z",
        "created_by": "A2", "revision": 2, "tasks": tasks,
        "status_summary": { "total": 4, "pending": 4, "in_progress": 0, "done": 0, "failed": 0, "skipped": 0 }
    })
}

/// First referee pass: R1 delivered, R2 absent from the evidence — an
/// honest unmet that triggers the revision loop.
fn compliance_reply_gap() -> Value {
    json!({
        "schema_version": "1.0", "spec_id": SPEC, "checked_by": "A1", "spec_match_pct": 50,
        "requirements": [
            { "id": "R1", "status": "met", "evidence": "tests/ac1.rs", "test_id": "AC1" },
            { "id": "R2", "status": "unmet", "evidence": "no versioned constant in artifacts" }
        ],
        "assumptions_audit": [{ "assumption_id": "AS1", "held": true }],
        "verdict": "needs_revision"
    })
}

fn compliance_reply_full() -> Value {
    json!({
        "schema_version": "1.0", "spec_id": SPEC, "checked_by": "A1", "spec_match_pct": 100,
        "requirements": [
            { "id": "R1", "status": "met", "evidence": "tests/ac1.rs", "test_id": "AC1" },
            { "id": "R2", "status": "met", "evidence": "src/versioned.rs" }
        ],
        "assumptions_audit": [{ "assumption_id": "AS1", "held": true }],
        "verdict": "approved"
    })
}

/// Golden mock answers for the REST-API form — sized for the schema.
fn rest_api_spec() -> Value {
    json!({
        "schema_version": "1.0",
        "id": SPEC,
        "version": "1.0.0",
        "created_at": "2026-09-12T09:00:00Z",
        "created_by": "A1",
        "source": {
            "raw_input_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "raw_input_path": "examples/spec_rest_api.md"
        },
        "title": "Minimal REST task API",
        "description": "axum service with list/create/complete endpoints.",
        "mode": "greenfield",
        "language": { "primary": "rust", "confidence": 1.0, "source": "explicit" },
        "requirements": [
            { "id": "R1", "text": "GET /tasks lists tasks", "priority": "must", "category": "functional" },
            { "id": "R2", "text": "POST /tasks creates a task with a title", "priority": "must", "category": "functional" },
            { "id": "R3", "text": "PATCH /tasks/:id completes a task", "priority": "must", "category": "functional" }
        ],
        "constraints": [{ "id": "C1", "text": "axum only, 127.0.0.1:8080" }],
        "acceptance_criteria": [
            { "id": "AC1", "text": "cargo test covers all three endpoints", "verifiable_by": "A4", "measurement": "exit_code == 0" }
        ],
        "out_of_scope": ["auth", "websockets"],
        "decisions": [{
            "id": "D1", "title": "framework", "choice": "axum",
            "alternatives_considered": [{ "option": "actix", "rejected_because": "form locks axum" }],
            "rationale": "explicit constraint C1", "evidence": ["form: Constraints"]
        }],
        "budget": { "max_cost_usd": 1.50, "max_duration_seconds": 7200 },
        "autonomy_level": 3,
        "approval_required": ["spec", "task_plan", "delivery"],
        "compliance_status": "approved"
    })
}

fn rest_api_plan() -> Value {
    json!({
        "schema_version": "1.0",
        "spec_id": SPEC,
        "created_at": "2026-09-12T09:05:00Z",
        "created_by": "A2",
        "revision": 1,
        "tasks": [
            { "id": "T1", "spec_requirement_ids": ["R1", "R2", "R3"], "title": "scaffold axum app",
              "description": "", "assigned_agent": "A5", "dependencies": [], "target": "hybrid",
              "acceptance": { "verifiable_by": "cargo", "criteria": "cargo check" }, "status": "pending" },
            { "id": "T2", "spec_requirement_ids": ["R1", "R2"], "title": "list+create handlers",
              "description": "", "assigned_agent": "A5", "dependencies": ["T1"], "target": "hybrid",
              "acceptance": { "verifiable_by": "cargo", "criteria": "cargo check" }, "status": "pending" },
            { "id": "T3", "spec_requirement_ids": ["R3"], "title": "complete handler",
              "description": "", "assigned_agent": "A5", "dependencies": ["T2"], "target": "hybrid",
              "acceptance": { "verifiable_by": "cargo", "criteria": "cargo check" }, "status": "pending" },
            { "id": "T4", "spec_requirement_ids": ["R1", "R2", "R3"], "title": "endpoint integration tests",
              "description": "", "assigned_agent": "A4", "dependencies": ["T3"], "target": "hybrid",
              "acceptance": { "verifiable_by": "cargo", "criteria": "cargo test" }, "status": "pending",
              "acceptance_ids": ["AC1"] }
        ],
        "status_summary": { "total": 4, "pending": 4, "in_progress": 0, "done": 0, "failed": 0, "skipped": 0 }
    })
}
