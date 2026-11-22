//! A2 — CEO (§5.6/§5.6A, KİLİTLİ). Turns the locked spec into a task DAG.
//! The LLM proposes task CONTENT and ORDER; the code enforces the contract —
//! nothing the model says about structure, identity or numbers is trusted:
//!
//! * §6.4 traceability — every requirement covered, no phantom citations
//!   (`check_traceability`, same gate `create_with_spec` enforces);
//! * the DAG must be schedulable (`Scheduler::from_plan`: cycles/unknown
//!   deps/duplicates rejected at PLANNING time, not mid-run);
//! * every `assigned_agent` must be executable (a plan naming an agent the
//!   bridge cannot dispatch is refused here, not at dispatch time);
//! * **"tahmin uydurmaz" (§5.6 rule 4)** — durations/costs come from
//!   `task_scores` history (Faz 8's owner), never from the model. The LLM's
//!   invented `estimated` numbers are STRIPPED to null (honest absence beats
//!   fabricated data; the pre-clamp then defers to the call-boundary wall);
//! * identity fields (`created_by`, `spec_id`, `revision`) and the derived
//!   views (`execution_order`, `critical_path`, `status_summary`) are written
//!   by CODE from the scheduler's truth — the model's versions are discarded.
//!
//! Not a DAG-dispatch agent: the control plane calls [`A2Ceo::make_plan`]
//! between the spec gate and the run's creation (§6.1 pre-scheduling hats).

use crate::config;
use crate::error::{Error, Result};
use crate::models::spec::Spec;
use crate::models::task::{TaskItem, TaskPlan};
use crate::repair::SchemaGuard;
use crate::scheduler::Scheduler;
use crate::validate;

pub struct A2Ceo;

impl A2Ceo {
    pub fn new() -> Self {
        Self
    }

    fn system_prefix() -> String {
        config::agents_prompt_from(std::path::Path::new("."), "A2_ceo")
            .unwrap_or_else(|_| "You are A2, the CEO planner.".into())
    }

    /// Schema-forced planning turn + full code-side enforcement. Errors are
    /// `Error::Agent` for contract violations the model must not slip past.
    /// Returns the plan AND the turn's honest usage (the pipeline charges
    /// planning turns like task turns — §8.2, no free management lunch).
    pub async fn make_plan(
        guard: &SchemaGuard,
        spec: &Spec,
    ) -> Result<(TaskPlan, crate::inference::TurnUsage)> {
        let req = crate::inference::InferenceRequest {
            task_type: "plan".into(),
            system_prefix: Self::system_prefix(),
            context: String::new(),
            input: format!(
                "Break this locked spec into an efficient task DAG (§5.6 rule 8: one agent, \
                 one file ownership, one session, >=1 requirement per task). Every requirement \
                 id in the spec MUST be cited by at least one task. Do NOT write estimates: \
                 leave `estimated` null (numbers come from history data, never from you).\n\n\
                 SPEC:\n{}",
                crate::hash::canonical_json(
                    &serde_json::to_value(spec).map_err(|e| Error::Agent(e.to_string()))?
                )
            ),
            max_tokens: Some(8192),
        };
        let out = guard.complete(&req, "tasks.schema.json").await?;
        let usage = crate::inference::TurnUsage::of(&out.response, out.turns);
        let mut plan: TaskPlan = serde_json::from_value(out.value)
            .map_err(|e| Error::Agent(format!("tasks document unreadable: {e}")))?;

        // Identity: the schema's `enum:["A2"]` pins created_by, `pattern` pins
        // the spec id's SHAPE — neither pins IDENTITY. Stamp both from the
        // spec so a model echoing a different (valid-looking) spec id dies
        // here, not as a mismatched ledger key mid-run.
        plan.created_by = "A2".into();
        plan.spec_id = spec.id.clone();
        plan.revision = plan.revision.max(1);

        // Dispatchability: fail at planning, not at execution. Naming a
        // not-yet-buildable agent (A6/A8…) is honest but unrunnable today.
        for t in &plan.tasks {
            if crate::agents::lookup(&t.assigned_agent).is_err() {
                return Err(Error::Agent(format!(
                    "plan assigns task {} to {} — not executable in the current registry",
                    t.id, t.assigned_agent
                )));
            }
        }

        // The DAG must actually run: cycles / unknown deps / duplicate ids are
        // scheduler-rejection material; check them on the FULL task list
        // first so the model gets one clean signal.
        let probe = TaskPlan {
            tasks: plan
                .tasks
                .iter()
                .map(|t| TaskItem {
                    status: "pending".into(),
                    ..t.clone()
                })
                .collect(),
            ..plan.clone()
        };
        let sched = Scheduler::from_plan(&probe, 1)
            .map_err(|e| Error::Agent(format!("plan DAG is not schedulable: {e}")))?;

        let (plan, stripped) = Self::enforce(plan, spec, &sched)?;
        let _ = stripped;
        Ok((plan, usage))
    }

    /// E2 scope-preserving replan (§5.6 m5): the CEO may re-cut the WORK,
    /// never the SPEC. The completed tasks of the current plan are
    /// untouchable — the ledger's rule-4 already refuses to re-run them, so
    /// a plan that drops or redefines them is rejected here (a model
    /// "revising" finished work is planning a lie). Requirement set must be
    /// identical (scope change is only via suggest + user).
    pub async fn replan(
        guard: &SchemaGuard,
        spec: &Spec,
        current: &TaskPlan,
        done_task_ids: &[String],
        why: &str,
    ) -> Result<(TaskPlan, crate::inference::TurnUsage)> {
        let req_ids: Vec<&str> = spec.requirements.iter().map(|r| r.id.as_str()).collect();
        let req_ids = req_ids.join(",");
        let reqs_before = spec.requirements.len();
        let req = crate::inference::InferenceRequest {
            task_type: "plan".into(),
            system_prefix: Self::system_prefix(),
            context: String::new(),
            input: format!(
                "REVISE the task plan (revision +1) so the run can meet its spec.                  Reason for the crisis: {why}
                 HARD RULES: the requirement set is UNCHANGED ({req_ids}); every DONE                  task {done_task_ids:?} must appear in the new plan byte-for-byte equal                  (id, assigned_agent, spec_requirement_ids) — completed work is never                  re-planned; leave `estimated` null (§5.6 rule 4).
                 SPEC:
{}
                 CURRENT PLAN:
{}",
                crate::hash::canonical_json(
                    &serde_json::to_value(spec).map_err(|e| Error::Agent(e.to_string()))?
                ),
                crate::hash::canonical_json(
                    &serde_json::to_value(current).map_err(|e| Error::Agent(e.to_string()))?
                )
            ),
            max_tokens: Some(8192),
        };
        let out = guard.complete(&req, "tasks.schema.json").await?;
        let usage = crate::inference::TurnUsage::of(&out.response, out.turns);
        let mut plan: TaskPlan = serde_json::from_value(out.value)
            .map_err(|e| Error::Agent(format!("revised plan unreadable: {e}")))?;
        plan.created_by = "A2".into();
        plan.spec_id = spec.id.clone();
        plan.revision = current.revision + 1;

        if spec.requirements.len() != reqs_before {
            return Err(Error::Agent(
                "replan cannot change the requirement set — scope shifts go through suggest".into(),
            ));
        }
        // Completed work is immutable — same id, same agent, same citations.
        for id in done_task_ids {
            let old =
                current.tasks.iter().find(|t| &t.id == id).ok_or_else(|| {
                    Error::Agent(format!("done task {id} absent from current plan"))
                })?;
            let new = plan.tasks.iter().find(|t| &t.id == id).ok_or_else(|| {
                Error::Agent(format!("replan dropped completed task {id} (§7.4 rule 4)"))
            })?;
            if new.assigned_agent != old.assigned_agent
                || new.spec_requirement_ids != old.spec_requirement_ids
            {
                return Err(Error::Agent(format!(
                    "replan redefined completed task {id}: done work is immutable"
                )));
            }
        }
        // Dispatchability for the NEW tasks; schedulability + traceability +
        // birth-pending + estimate-stripping through the shared core.
        for t in &plan.tasks {
            if crate::agents::lookup(&t.assigned_agent).is_err() {
                return Err(Error::Agent(format!(
                    "plan assigns task {} to {} — not executable in the current registry",
                    t.id, t.assigned_agent
                )));
            }
        }
        let probe = TaskPlan {
            tasks: plan
                .tasks
                .iter()
                .map(|t| TaskItem {
                    status: "pending".into(),
                    ..t.clone()
                })
                .collect(),
            ..plan.clone()
        };
        let sched = Scheduler::from_plan(&probe, 1)
            .map_err(|e| Error::Agent(format!("revised DAG is not schedulable: {e}")))?;
        let (plan, _) = Self::enforce(plan, spec, &sched)?;
        Ok((plan, usage))
    }

    /// The contract core shared by make_plan and replan: born-pending,
    /// invented estimates stripped, derived views recomputed from the
    /// scheduler's truth. Also returns how many estimates were stripped.
    fn enforce(plan: TaskPlan, spec: &Spec, sched: &Scheduler) -> Result<(TaskPlan, usize)> {
        let problems = validate::check_traceability(spec, &plan);
        if !problems.is_empty() {
            return Err(Error::Traceability(problems.join("; ")));
        }
        let mut plan = plan;
        // A plan is born pending: a model writing "done" into its own fresh
        // plan would poison plan.json (the DB rows are already forced
        // pending by `create` — the on-disk document must tell the same truth).
        for t in &mut plan.tasks {
            t.status = "pending".into();
        }
        // §5.6 rule 4: the model's invented estimates are stripped, never
        // adopted. task_scores (the only sanctioned source) is empty until
        // Faz 8 — an invented number is a lie by construction today.
        let stripped = plan.tasks.iter().filter(|t| t.estimated.is_some()).count();
        for t in &mut plan.tasks {
            t.estimated = None;
        }
        if stripped > 0 {
            tracing::warn!(
                agent = "A2",
                stripped,
                "plan carried LLM-invented estimates — forced to null (§5.6 rule 4)"
            );
        }

        // Derived views: CODE truth replaces whatever the model wrote.
        plan.execution_order = sched.topological_order();
        plan.critical_path = sched.critical_path();
        plan.status_summary = sched.status_summary();
        Ok((plan, stripped))
    }
}

impl Default for A2Ceo {
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

    fn guard_with(text: &str) -> (SchemaGuard, Arc<MockProvider>) {
        let mock = Arc::new(MockProvider::new(vec![MockStep::Success {
            text: text.into(),
            tokens_in: 300,
            tokens_out: 150,
        }]));
        (SchemaGuard::new(mock.clone(), repo_root()), mock)
    }

    fn spec() -> Spec {
        let doc =
            std::fs::read_to_string(repo_root().join("examples/expected/spec_valid.json")).unwrap();
        serde_json::from_str(&doc).unwrap()
    }

    fn plan_json() -> serde_json::Value {
        let doc = std::fs::read_to_string(repo_root().join("examples/expected/tasks_valid.json"))
            .unwrap();
        serde_json::from_str(&doc).unwrap()
    }

    #[tokio::test]
    async fn accepted_plan_gets_code_stamped_identity_and_derived_views() {
        let (guard, _m) = guard_with(&plan_json().to_string());
        let (plan, _usage) = A2Ceo::make_plan(&guard, &spec()).await.unwrap();
        assert_eq!(plan.created_by, "A2");
        assert_eq!(plan.spec_id, "spec-20260912-001");
        assert!(plan.revision >= 1);
        // Derived views are the scheduler's truth, not the model's prose.
        assert_eq!(
            plan.execution_order,
            vec!["T1".to_string(), "T2".to_string()]
        );
        assert!(!plan.critical_path.is_empty());
        assert_eq!(plan.status_summary.total, 2);
        assert_eq!(plan.status_summary.pending, 2);
    }

    #[tokio::test]
    async fn llm_invented_estimates_are_stripped_never_adopted() {
        // §5.6 rule 4 proof: the positive fixture has a REAL estimated block
        // on T1 — a model copy-pasting it into a plan is fabricating a number
        // no data backs. make_plan must null it while keeping the task.
        let mut doc = plan_json();
        doc["tasks"][0]["estimated"]["cost_usd"] = json!(42.0);
        doc["tasks"][1]["estimated"] = json!({ "cost_usd": 99.0, "duration_seconds": 9 });
        let (guard, _) = guard_with(&doc.to_string());
        let (plan, _) = A2Ceo::make_plan(&guard, &spec()).await.unwrap();
        assert!(plan.tasks.iter().all(|t| t.estimated.is_none()));
    }

    fn current_plan() -> TaskPlan {
        serde_json::from_value(plan_json()).unwrap()
    }

    #[tokio::test]
    async fn replan_adds_recovery_work_and_bumps_revision() {
        let mut doc = plan_json();
        doc["revision"] = json!(9); // claimed; code recomputes old+1
        doc["tasks"].as_array_mut().unwrap().push(json!({
            "id": "T3", "spec_requirement_ids": ["R2"], "title": "recovery for R2",
            "description": "", "assigned_agent": "A5", "dependencies": ["T2"],
            "target": "cloud", "acceptance": {"verifiable_by": "A4", "criteria": "covers R2"},
            "status": "pending"
        }));
        doc["status_summary"]["total"] = json!(3);
        let (guard, _) = guard_with(&doc.to_string());
        let (plan, usage) = A2Ceo::replan(
            &guard,
            &spec(),
            &current_plan(),
            &["T1".to_string()],
            "unmet: R2",
        )
        .await
        .unwrap();
        assert_eq!(
            plan.revision,
            current_plan().revision + 1,
            "revision is CODE's"
        );
        assert_eq!(plan.tasks.len(), 3);
        assert_eq!(usage.tokens_in, 300);
        // T1 kept intact & done in DB-truth sense (status handled by caller);
        // execution order derived by code covers the new node.
        assert!(plan.execution_order.iter().any(|id| id == "T3"));
    }

    #[tokio::test]
    async fn replan_cannot_drop_completed_tasks() {
        // The classic model lie: "re-plan" as if nothing had happened.
        let mut doc = plan_json();
        doc["tasks"] = json!([{
            "id": "T2", "spec_requirement_ids": ["R1", "R2"], "title": "rewrite everything",
            "description": "", "assigned_agent": "A5", "dependencies": [],
            "target": "cloud", "acceptance": {"verifiable_by": "A4", "criteria": "x"},
            "status": "pending"
        }]);
        doc["status_summary"]["total"] = json!(1);
        let (guard, _) = guard_with(&doc.to_string());
        let err = A2Ceo::replan(
            &guard,
            &spec(),
            &current_plan(),
            &["T1".to_string()],
            "unmet: R2",
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, Error::Agent(m) if m.contains("dropped completed")),
            "{err}"
        );
    }

    #[tokio::test]
    async fn replan_cannot_redefine_completed_tasks() {
        let mut doc = plan_json();
        doc["tasks"][0]["assigned_agent"] = json!("A4"); // T1 was A5, done
        let (guard, _) = guard_with(&doc.to_string());
        let err = A2Ceo::replan(
            &guard,
            &spec(),
            &current_plan(),
            &["T1".to_string()],
            "unmet: R2",
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, Error::Agent(m) if m.contains("done work is immutable")),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_fresh_plan_is_born_pending_whatever_the_model_claims() {
        let mut doc = plan_json();
        doc["tasks"][0]["status"] = json!("done"); // claiming its own delivery
        let (guard, _) = guard_with(&doc.to_string());
        let (plan, _) = A2Ceo::make_plan(&guard, &spec()).await.unwrap();
        assert!(plan.tasks.iter().all(|t| t.status == "pending"));
        assert_eq!(plan.status_summary.pending, 2);
        assert_eq!(plan.status_summary.done, 0);
    }

    #[tokio::test]
    async fn plan_leaving_a_requirement_uncovered_is_rejected() {
        let mut doc = plan_json();
        doc["tasks"] = json!([{
            "id": "T1", "spec_requirement_ids": ["R1"], "title": "only R1",
            "description": "", "assigned_agent": "A5",
            "dependencies": [], "target": "cloud",
            "acceptance": {"verifiable_by": "A4", "criteria": "x"}, "status": "pending"
        }]);
        doc["status_summary"]["total"] = json!(1);
        doc["execution_order"] = json!(["T1"]);
        doc["critical_path"] = json!(["T1"]);
        let (guard, _) = guard_with(&doc.to_string());
        let err = A2Ceo::make_plan(&guard, &spec()).await.unwrap_err();
        assert!(
            matches!(&err, Error::Traceability(m) if m.contains("TASK_UNCOVERED_REQUIREMENT: R2")),
            "{err}"
        );
    }

    #[tokio::test]
    async fn plan_with_cycle_is_rejected_as_unschedulable() {
        let mut doc = plan_json();
        doc["tasks"][0]["dependencies"] = json!(["T2"]); // T1 ⇄ T2
        let (guard, _) = guard_with(&doc.to_string());
        let err = A2Ceo::make_plan(&guard, &spec()).await.unwrap_err();
        assert!(
            matches!(&err, Error::Agent(m) if m.contains("not schedulable")),
            "{err}"
        );
    }

    #[tokio::test]
    async fn plan_naming_an_unbuildable_agent_is_refused_at_planning() {
        let mut doc = plan_json();
        doc["tasks"][0]["assigned_agent"] = json!("A8"); // valid id, no runtime
        let (guard, _) = guard_with(&doc.to_string());
        let err = A2Ceo::make_plan(&guard, &spec()).await.unwrap_err();
        assert!(
            matches!(&err, Error::Agent(m) if m.contains("not executable")),
            "{err}"
        );
    }

    #[tokio::test]
    async fn foreign_spec_id_cannot_survive_the_gate() {
        // A valid-shape but DIFFERENT spec id is an identity forgery, not a
        // format error — the schema passes it; the code must not.
        let mut doc = plan_json();
        doc["spec_id"] = json!("spec-20250101-999");
        let (guard, _) = guard_with(&doc.to_string());
        let (plan, _) = A2Ceo::make_plan(&guard, &spec()).await.unwrap();
        assert_eq!(
            plan.spec_id,
            spec().id,
            "code re-stamps identity from the spec"
        );
    }
}
