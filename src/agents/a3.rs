//! A3 — Chief Reporter (§5.7, KİLİTLİ). The system's sole reporting owner:
//! the NUMBERS come from the system of record (state.db + events + result
//! files), the LLM only ever writes narrative. Enforced structurally:
//!
//! * [`A3Reporter::skeleton`] is pure code over [`RunFacts`] — every figure
//!   in the report is derived from what ACTUALLY ran (P2 discipline: a red
//!   task set can never get a green summary);
//! * [`A3Reporter::narrate`] sends the skeleton to the model but merges BACK
//!   only the narrative fields (`executive_summary`, `next_steps`,
//!   `warnings` additions) — a model that rewrites `cost.total_usd` is
//!   ignored, not adopted. Numbers do not flow upward from the LLM (§5.7 m2);
//! * [`A3Reporter::seal`] embeds A1's adjudicated compliance verdict — the
//!   report is complete BEFORE the delivery gate is passed (§5.7 m4).

use serde_json::Value;

use crate::config;
use crate::error::{Error, Result};
use crate::models::compliance::ComplianceCheck;
use crate::models::report::{
    ArtifactGroups, Compliance, Cost, CostBreakdown, DepartmentReport, ReportSummary,
    RequirementStatus, TaskRecord, Tokens, TraceEntry,
};
use crate::models::spec::Spec;
use crate::models::task::TaskPlan;
use crate::repair::SchemaGuard;

/// Per-task ground truth as CODE observed it — A3 never re-reads agent prose.
#[derive(Debug, Clone, Default)]
pub struct TaskFact {
    pub id: String,
    pub agent: String,
    /// Scheduler truth: `done` | `failed` | `blocked` | `skipped` | `pending`.
    pub status: String,
    pub artifacts: Vec<String>,
    pub errors: Vec<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cached_tokens: u64,
    pub cost_usd: f64,
    pub cache_savings_usd: f64,
    pub duration_seconds: Option<u64>,
}

/// Everything the deterministic skeleton is computed from — injected by the
/// pipeline from state.db / events (never from the model).
#[derive(Debug, Clone)]
pub struct RunFacts<'a> {
    pub spec: &'a Spec,
    pub plan: &'a TaskPlan,
    pub tasks: Vec<TaskFact>,
    pub spent_usd: f64,
    pub budget_usd: f64,
    pub duration_seconds: u64,
    pub retries: u32,
    pub escalations: u32,
    pub generated_at: String,
}

pub struct A3Reporter;

impl A3Reporter {
    pub fn new() -> Self {
        Self
    }

    fn system_prefix() -> String {
        config::agents_prompt_from(std::path::Path::new("."), "A3_reporter")
            .unwrap_or_else(|_| "You are A3, the Chief Reporter.".into())
    }

    /// The full numeric/analytic skeleton. Pure — golden-testable without a
    /// provider (§5.7 m2: "sayısal bölüm koddan").
    pub fn skeleton(f: &RunFacts) -> DepartmentReport {
        let done = f.tasks.iter().filter(|t| t.status == "done").count();
        let failed = f.tasks.iter().filter(|t| t.status == "failed").count();
        // The schema has no `blocked` counter; blocked work was NOT run, like
        // skipped — counted honestly under skipped, with a warning line.
        let skipped = f
            .tasks
            .iter()
            .filter(|t| matches!(t.status.as_str(), "skipped" | "blocked"))
            .count();
        let final_status = if done == f.tasks.len() && !f.tasks.is_empty() {
            "success"
        } else if done > 0 {
            "partial"
        } else {
            "failed"
        };

        let mut breakdown = CostBreakdown::default();
        let mut cache_savings = 0.0;
        let mut unbucketed = 0.0;
        for t in &f.tasks {
            cache_savings += t.cache_savings_usd;
            match t.provider.as_deref() {
                Some("openrouter") | Some("cloud") => breakdown.cloud_llm += t.cost_usd,
                Some("local") => breakdown.local_inference += t.cost_usd,
                // `mock` is the §21.1 test double and bills $0 — bucketing a
                // zero would be noise, but ANY nonzero unbucketed spend is a
                // hole in the accounting and must be surfaced as a warning.
                _ => unbucketed += t.cost_usd,
            }
        }

        let mut warnings: Vec<String> = Vec::new();
        if unbucketed > 0.0 {
            warnings.push(format!(
                "{unbucketed:.6} USD of spend matched no provider bucket — totals include it \
                 but the breakdown under-reports"
            ));
        }
        if skipped > 0 {
            warnings.push(format!(
                "{skipped} task(s) never ran (blocked/skipped) — counted under tasks_skipped"
            ));
        }

        let mut artifacts = ArtifactGroups::default();
        for t in &f.tasks {
            for p in &t.artifacts {
                if p.starts_with("tests/") {
                    artifacts.tests.push(p.clone());
                } else if p.ends_with(".md") {
                    artifacts.docs.push(p.clone());
                } else if p.starts_with("src/") || p.ends_with(".rs") {
                    artifacts.source_files.push(p.clone());
                } else {
                    artifacts.devops.push(p.clone());
                }
            }
        }

        // §6.4 map: requirement -> citing tasks -> outcome CODE can observe.
        let traceability: Vec<TraceEntry> = f
            .spec
            .requirements
            .iter()
            .map(|r| {
                let citing: Vec<&TaskFact> = f
                    .tasks
                    .iter()
                    .filter(|t| {
                        f.plan
                            .tasks
                            .iter()
                            .find(|p| p.id == t.id)
                            .map(|p| p.spec_requirement_ids.iter().any(|q| q == &r.id))
                            .unwrap_or(false)
                    })
                    .collect();
                let all_done = !citing.is_empty() && citing.iter().all(|t| t.status == "done");
                let evidence = citing
                    .iter()
                    .find_map(|t| t.artifacts.first())
                    .cloned()
                    .unwrap_or_else(|| "no artifact produced".into());
                let test_id = citing
                    .iter()
                    .find_map(|t| t.artifacts.iter().find(|p| p.starts_with("tests/")))
                    .cloned();
                TraceEntry {
                    requirement_id: r.id.clone(),
                    task_ids: citing.iter().map(|t| t.id.clone()).collect(),
                    evidence: Some(evidence),
                    test_id,
                    status: if all_done { "met" } else { "unmet" }.into(),
                }
            })
            .collect();

        let tasks: Vec<TaskRecord> = f
            .tasks
            .iter()
            .map(|t| TaskRecord {
                id: t.id.clone(),
                agent: t.agent.clone(),
                status: t.status.clone(),
                duration_seconds: t.duration_seconds,
                tokens: Some(Tokens {
                    in_tokens: t.tokens_in,
                    out_tokens: t.tokens_out,
                    cached_tokens: t.cached_tokens,
                }),
                cost_usd: Some(t.cost_usd),
                model: t.model.clone(),
                artifacts: t.artifacts.clone(),
                errors: t.errors.clone(),
            })
            .collect();

        DepartmentReport {
            schema_version: "1.0".into(),
            spec_id: f.spec.id.clone(),
            generated_at: f.generated_at.clone(),
            generated_by: "A3".into(),
            final_status: final_status.into(),
            duration_seconds: f.duration_seconds,
            summary: ReportSummary {
                tasks_total: f.tasks.len() as u32,
                tasks_done: done as u32,
                tasks_failed: failed as u32,
                tasks_skipped: skipped as u32,
                retries: f.retries,
                escalations: f.escalations,
            },
            cost: Cost {
                total_usd: f.spent_usd,
                breakdown,
                cache_savings_usd: cache_savings,
                budget_limit_usd: f.budget_usd,
                budget_used_pct: if f.budget_usd > 0.0 {
                    f.spent_usd / f.budget_usd * 100.0
                } else {
                    0.0
                },
            },
            quality_tradeoff: vec![],
            tasks,
            artifacts,
            traceability,
            compliance: Compliance::default(),
            idea_audit: vec![],
            health_notes: vec![],
            executive_summary: None,
            warnings,
            next_steps: vec![match final_status {
                "success" => "Human review of the sealed report, then delivery gate.".into(),
                _ => "Inspect the failed/unrun tasks before retrying.".into(),
            }],
            extra: Default::default(),
        }
    }

    /// Narrative-only LLM turn. The model sees the skeleton (structured data,
    /// never raw logs — §5.7 m1) and may phrase the story; the code keeps
    /// ONLY the narrative fields from its answer. A provider failure degrades
    /// to no narrative (the numbers ARE the product) — visibly, via warn.
    pub async fn narrate(
        guard: &SchemaGuard,
        report: &DepartmentReport,
    ) -> Option<(DepartmentReport, crate::inference::TurnUsage)> {
        let skeleton = serde_json::to_value(report).ok()?;
        let req = crate::inference::InferenceRequest {
            task_type: "report".into(),
            system_prefix: Self::system_prefix(),
            context: String::new(),
            input: format!(
                "Write the narrative for this run report. Every numeric field below is \
                 ground truth from the system of record: DO NOT change any number — return \
                 the document with `executive_summary` (≤1200 chars, plain-language outcome \
                 for the user) and any genuinely useful `warnings`/`next_steps` additions.\n\n\
                 REPORT SKELETON:\n{}",
                crate::hash::canonical_json(&skeleton)
            ),
            max_tokens: Some(2048),
        };
        match guard.complete(&req, "department_report.schema.json").await {
            Ok(out) => {
                // Merge: narrative fields from the model, EVERYTHING else from
                // the code-built skeleton. Values are re-derived, not trusted.
                let v = out.value.clone();
                let usage = crate::inference::TurnUsage::of(&out.response, out.turns);
                let mut merged = report.clone();
                if let Some(s) = v.get("executive_summary").and_then(Value::as_str) {
                    merged.executive_summary = Some(s.to_string());
                }
                if let Some(extra_steps) = v.get("next_steps").and_then(Value::as_array) {
                    for step in extra_steps.iter().filter_map(|x| x.as_str()) {
                        if !merged.next_steps.iter().any(|s| s == step) {
                            merged.next_steps.push(step.to_string());
                        }
                    }
                }
                if let Some(extra_warn) = v.get("warnings").and_then(Value::as_array) {
                    for w in extra_warn.iter().filter_map(|x| x.as_str()) {
                        if !merged.warnings.iter().any(|s| s == w) {
                            merged.warnings.push(w.to_string());
                        }
                    }
                }
                Some((merged, usage))
            }
            Err(e) => {
                tracing::warn!(
                    agent = "A3",
                    "narrative turn failed, shipping numeric-only: {e}"
                );
                None
            }
        }
    }

    /// Seal step (§5.7 m4): A1's adjudicated verdict is embedded, and the
    /// report's own traceability statuses must not contradict the referee —
    /// code resolves any disagreement in the STRICTER direction.
    pub fn seal(report: &mut DepartmentReport, cc: &ComplianceCheck) {
        report.compliance = Compliance {
            checked_by: "A1".into(),
            spec_match_pct: cc.spec_match_pct,
            requirements: cc
                .requirements
                .iter()
                .map(|r| RequirementStatus {
                    id: r.id.clone(),
                    status: r.status.clone(),
                    evidence: Some(r.evidence.clone()),
                })
                .collect(),
            unmet_requirements: cc.unmet_requirements.clone(),
            acceptance_criteria_passed: cc
                .requirements
                .iter()
                .filter(|r| r.status == "met")
                .filter_map(|r| r.test_id.clone())
                .collect(),
        };
        for trace in &mut report.traceability {
            if let Some(v) = cc
                .requirements
                .iter()
                .find(|r| r.id == trace.requirement_id)
            {
                if v.status == "met" && trace.status != "met" {
                    // The referee cites evidence the skeleton could not see —
                    // upgrade with the evidence noted, never the reverse for
                    // unmet: a met-by-code claim cannot erase an unmet verdict.
                    trace.status = v.status.clone();
                    trace.evidence = Some(format!(
                        "A1: {}",
                        if v.evidence.is_empty() {
                            "see compliance"
                        } else {
                            &v.evidence
                        }
                    ));
                } else if v.status != "met" {
                    trace.status = v.status.clone();
                }
            }
        }
    }

    /// Canonical bytes for `runs/<spec_id>/report.json` — the sealed artifact.
    pub fn to_canonical_json(report: &DepartmentReport) -> Result<Value> {
        serde_json::to_value(report).map_err(|e| Error::Agent(format!("report serialize: {e}")))
    }

    /// Final honest self-check: the sealed report must agree with the task
    /// truth it summarizes (P2 discipline — a green story over red work,
    /// or over an unmet spec, is rejected).
    pub fn agrees_with_work(report: &DepartmentReport, all_done: bool) -> bool {
        if !all_done && report.final_status == "success" {
            return false;
        }
        // Sealed (A1 present) + sub-100: the run did not meet its spec, and
        // the report may not read as a clean success whatever the task
        // statuses say (§5.4C assumption/requirement audit feeds here).
        let sealed = report.compliance.checked_by == "A1";
        if sealed && report.compliance.spec_match_pct < 100 && report.final_status == "success" {
            return false;
        }
        true
    }
}

impl Default for A3Reporter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{MockProvider, MockStep};
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn spec() -> Spec {
        let doc =
            std::fs::read_to_string(repo_root().join("examples/expected/spec_valid.json")).unwrap();
        serde_json::from_str(&doc).unwrap()
    }

    fn plan() -> TaskPlan {
        let doc = std::fs::read_to_string(repo_root().join("examples/expected/tasks_valid.json"))
            .unwrap();
        serde_json::from_str(&doc).unwrap()
    }

    fn task(id: &str, agent: &str, status: &str, cost: f64, provider: &str) -> TaskFact {
        TaskFact {
            id: id.into(),
            agent: agent.into(),
            status: status.into(),
            provider: Some(provider.into()),
            cost_usd: cost,
            ..Default::default()
        }
    }

    #[test]
    fn skeleton_counts_and_buckets_come_from_task_truth() {
        let s = spec();
        let p = plan();
        let f = RunFacts {
            spec: &s,
            plan: &p,
            tasks: vec![
                task("T1", "A5", "done", 0.01, "openrouter"),
                task("T2", "A7", "done", 0.0, "local"),
                task("T3", "A4", "blocked", 0.0, "mock"),
            ],
            spent_usd: 0.01,
            budget_usd: 0.02,
            duration_seconds: 90,
            retries: 1,
            escalations: 0,
            generated_at: "2026-09-12T10:00:00Z".into(),
        };
        let r = A3Reporter::skeleton(&f);
        assert_eq!(r.generated_by, "A3");
        assert_eq!(r.summary.tasks_total, 3);
        assert_eq!(r.summary.tasks_done, 2);
        assert_eq!(r.summary.tasks_skipped, 1, "blocked counts as never-ran");
        assert_eq!(
            r.final_status, "partial",
            "one task never ran — not success"
        );
        assert_eq!(r.cost.breakdown.cloud_llm, 0.01);
        assert_eq!(r.cost.breakdown.local_inference, 0.0);
        assert_eq!(r.cost.budget_used_pct, 50.0);
        assert!(r.warnings.iter().any(|w| w.contains("blocked/skipped")));
        // Traceability cites the plan's citations, code-derived.
        let r1 = r
            .traceability
            .iter()
            .find(|t| t.requirement_id == "R1")
            .unwrap();
        assert!(r1.task_ids.contains(&"T1".to_string()));
    }

    #[tokio::test]
    async fn narrative_may_phrase_but_never_rewrite_the_numbers() {
        // The model returns the skeleton with total_usd=999 and a fake
        // spec_id — the merged report keeps CODE's 0.01 and identity.
        let s = spec();
        let p = plan();
        let f = RunFacts {
            spec: &s,
            plan: &p,
            tasks: vec![task("T1", "A5", "done", 0.01, "openrouter")],
            spent_usd: 0.01,
            budget_usd: 1.0,
            duration_seconds: 10,
            retries: 0,
            escalations: 0,
            generated_at: "2026-09-12T10:00:00Z".into(),
        };
        let report = A3Reporter::skeleton(&f);
        // The model lies in EVERY field it might touch: status, counts, cost.
        let lied = json!({
            "schema_version": "1.0",
            "spec_id": "spec-20260912-001",
            "generated_at": "2026-09-12T10:00:00Z",
            "generated_by": "A3",
            "final_status": "failed",
            "summary": {
                "tasks_total": 99, "tasks_done": 99, "tasks_failed": 0,
                "tasks_skipped": 0, "retries": 0, "escalations": 0
            },
            "cost": {
                "total_usd": 999.0, "breakdown": {}, "cache_savings_usd": 0.0,
                "budget_limit_usd": 1.0, "budget_used_pct": 99900.0
            },
            "executive_summary": "The run finished cleanly; one task delivered.",
            "next_steps": ["Archive the run."]
        });
        let mock = Arc::new(MockProvider::new(vec![MockStep::Success {
            text: lied.to_string(),
            tokens_in: 100,
            tokens_out: 40,
        }]));
        let guard = SchemaGuard::new(mock.clone(), repo_root());
        let (merged, _) = A3Reporter::narrate(&guard, &report)
            .await
            .expect("narrative ok");
        assert_eq!(
            merged.cost.total_usd, 0.01,
            "numbers come from code, not the model"
        );
        assert_eq!(merged.summary.tasks_total, 1);
        assert_eq!(merged.final_status, "success", "truth: 1/1 tasks done");
        assert!(
            merged
                .executive_summary
                .as_deref()
                .unwrap()
                .contains("finished cleanly"),
            "narrative fields DO come from the model"
        );
        assert!(merged.next_steps.contains(&"Archive the run.".to_string()));

        // And: when the narrative turn FAILS, the skeleton ships anyway.
        let exhausted = Arc::new(MockProvider::new(vec![]));
        let guard2 = SchemaGuard::new(exhausted, repo_root());
        assert!(A3Reporter::narrate(&guard2, &report).await.is_none());
    }

    #[test]
    fn seal_embeds_referee_and_never_loosens_unmet() {
        let s = spec();
        let p = plan();
        let mut report = {
            let f = RunFacts {
                spec: &s,
                plan: &p,
                tasks: vec![task("T1", "A5", "done", 0.0, "mock")],
                spent_usd: 0.0,
                budget_usd: 1.0,
                duration_seconds: 5,
                retries: 0,
                escalations: 0,
                generated_at: "2026-09-12T10:00:00Z".into(),
            };
            A3Reporter::skeleton(&f)
        };
        let cc: ComplianceCheck = serde_json::from_value(json!({
            "schema_version": "1.0", "spec_id": s.id, "checked_by": "A1",
            "spec_match_pct": 50,
            "requirements": [
                {"id": "R1", "status": "met", "evidence": "src/routes.rs:12", "test_id": "AC1"},
                {"id": "R2", "status": "unmet", "evidence": "missing endpoint"}
            ],
            "unmet_requirements": ["R2"],
            "verdict": "needs_revision"
        }))
        .unwrap();
        // (The hand-built cc is already final — production inputs arrive
        // through A1's `adjudicate`, which is covered in a1's own tests.)
        A3Reporter::seal(&mut report, &cc);
        assert_eq!(report.compliance.checked_by, "A1");
        assert_eq!(report.compliance.spec_match_pct, 50);
        assert_eq!(report.compliance.unmet_requirements, vec!["R2".to_string()]);
        assert_eq!(
            report.compliance.acceptance_criteria_passed,
            vec!["AC1".to_string()]
        );
        let r2 = report
            .traceability
            .iter()
            .find(|t| t.requirement_id == "R2")
            .unwrap();
        assert_eq!(
            r2.status, "unmet",
            "a met-by-code trace cannot survive an unmet verdict"
        );
        // P2: a success story over a sub-100 seal is rejected.
        report.final_status = "success".into();
        assert!(!A3Reporter::agrees_with_work(&report, false));
    }
}
