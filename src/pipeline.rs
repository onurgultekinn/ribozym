//! The Faz 5-c/5-d glue: the control-plane hats (§6.1) wired to the
//! orchestrator's two seams. Async agent turns (A1/A2/A3) are driven from
//! the synchronous driver exactly like the bridge (§5.3): a dedicated
//! current-thread runtime per call — the control plane never becomes async,
//! so golden determinism (§21.2) is untouched.
//!
//! Every model turn goes through [`Orchestrator::run_op`] (kind `plan_turn`):
//! deterministic op identity from the input payload, persisted result
//! envelope, exactly-once charge (§8.2), and §7.4 rule-4 recovery — a
//! finished turn is never re-invoiced. The truth documents (`spec.json`,
//! `plan.json`, `report.json`, `compliance.json`) live under
//! `runs/<spec_id>/` and are READ BACK from disk, never cached as authority:
//! resume rebuilds state from what actually persisted (§7.1 single source).

use std::path::{Path, PathBuf};

use crate::agents::{A1Architect, A2Ceo, A3Reporter, RunFacts, TaskFact};
use crate::clock;
use crate::error::{Error, Result};
use crate::events::EventLog;
use crate::hash;
use crate::models::compliance::ComplianceCheck;
use crate::models::event::EventKind as Ev;
use crate::models::report::DepartmentReport;
use crate::models::spec::Spec;
use crate::models::task::TaskPlan;
use crate::orchestrator::{
    FinalPipeline, Orchestrator, PlanningPipeline, ReplanOutcome, RunConfig,
};
use crate::repair::SchemaGuard;
use crate::state::StateStore;
use crate::validate;

/// Drive one async hat call to completion on its own current-thread runtime
/// (the bridge's seam decision, applied to the planning agents).
fn block_on_agent<F: std::future::Future>(f: F) -> Result<F::Output> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::Agent(format!("runtime: {e}")))
        .map(|rt| rt.block_on(f))
}

/// Canonical bytes for a run document (`runs/<spec_id>/<name>.json`).
/// Canonical JSON keeps the on-disk golden artifacts byte-stable (§21.3).
pub fn write_doc(
    root: &Path,
    spec_id: &str,
    name: &str,
    doc: &serde_json::Value,
) -> Result<PathBuf> {
    let dir = root.join("runs").join(spec_id);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{name}.json"));
    std::fs::write(&path, hash::canonical_json(doc))?;
    Ok(path)
}

pub struct AgentPipeline {
    cfg: RunConfig,
    /// Explicit dependency (repo root holding `schemas/`) — no environment
    /// probing at validation time; the caller OWNS this path.
    schema_dir: PathBuf,
    guard: std::sync::Arc<SchemaGuard>,
    /// The user's form bytes, held until `save_form` persists them as the
    /// run's provenance file (`source.raw_input_path` points there).
    form: Vec<u8>,
    /// The report between Finalizing and seal; None before finalize ran.
    report: Option<DepartmentReport>,
    /// The last adjudicated verdict — the replanner's ONLY view of why the
    /// run is in crisis (never re-asked from the model, §7.1).
    last_verdict: Option<ComplianceCheck>,
}

impl AgentPipeline {
    pub fn for_form(
        cfg: RunConfig,
        schema_dir: impl Into<PathBuf>,
        guard: std::sync::Arc<SchemaGuard>,
        form: Vec<u8>,
    ) -> Self {
        Self {
            cfg,
            schema_dir: schema_dir.into(),
            guard,
            form,
            report: None,
            last_verdict: None,
        }
    }

    pub fn for_resume(
        cfg: RunConfig,
        schema_dir: impl Into<PathBuf>,
        guard: std::sync::Arc<SchemaGuard>,
    ) -> Self {
        Self::for_form(cfg, schema_dir, guard, Vec::new())
    }

    fn rundir(&self) -> PathBuf {
        self.cfg.root.join("runs").join(&self.cfg.spec_id)
    }

    fn doc_path(&self, name: &str) -> PathBuf {
        self.rundir().join(format!("{name}.json"))
    }

    fn archive(&self) -> crate::archive::Archive {
        crate::archive::Archive::open(&self.cfg.root, &self.cfg.spec_id)
    }

    /// §7.6 B2: every lifecycle read is verified against the archive
    /// manifest (when the section has been sealed). A drift is an error,
    /// never a silent pass; the run parks for repair or the human.
    fn verify_against_archive(&self, label: &str, bytes: &[u8]) -> Result<()> {
        if !self.archive().matches_any(label, bytes)? {
            return Err(Error::ArchiveCorrupt(format!(
                "{label}: document matches no sealed archive section"
            )));
        }
        Ok(())
    }

    /// The validated spec — READ FROM DISK. A missing document before its
    /// phase ran is a wiring bug, not an empty default (§7.1).
    pub fn spec(&self) -> Result<Spec> {
        let raw = std::fs::read(self.doc_path("spec")).map_err(|e| {
            Error::Agent(format!(
                "spec.json unreadable (draft_spec must run first): {e}"
            ))
        })?;
        self.verify_against_archive("spec.json", &raw)?;
        serde_json::from_slice(&raw).map_err(|e| Error::Agent(format!("spec.json corrupt: {e}")))
    }

    /// The plan — same disk-first rule.
    pub fn plan(&self) -> Result<TaskPlan> {
        let raw = std::fs::read(self.doc_path("plan")).map_err(|e| {
            Error::Agent(format!(
                "plan.json unreadable (plan_tasks must run first): {e}"
            ))
        })?;
        self.verify_against_archive("plan.json", &raw)?;
        serde_json::from_slice(&raw).map_err(|e| Error::Agent(format!("plan.json corrupt: {e}")))
    }

    /// Read the ground truth A3 reports FROM: tasks table + event summaries
    /// + runs row. The model is consulted for narrative only (§5.7).
    ///
    /// The spec/plan are caller-loaded (disk-first); facts borrow them.
    fn collect_facts<'a>(&self, spec: &'a Spec, plan: &'a TaskPlan) -> Result<RunFacts<'a>> {
        let store = StateStore::open(&self.cfg.root.join(".ribozym").join("state.db"))?;
        let conn = store.conn();
        let (spent, budget, created_at, updated_at): (f64, f64, String, String) = conn.query_row(
            "SELECT spent_usd, budget_usd, created_at, updated_at FROM runs WHERE spec_id = ?1",
            rusqlite::params![self.cfg.spec_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        let duration_seconds = match (
            clock::epoch_from_utc_iso(&created_at),
            clock::epoch_from_utc_iso(&updated_at),
        ) {
            (Some(a), Some(b)) if b > a => b - a,
            _ => 0,
        };
        let rows: Vec<(String, String, String, i64)> = {
            let mut stmt = conn.prepare(
                "SELECT task_id, status, agent, retry_count FROM tasks WHERE spec_id = ?1 ORDER BY task_id",
            )?;
            let it = stmt.query_map(rusqlite::params![self.cfg.spec_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?;
            it.collect::<rusqlite::Result<Vec<_>>>()?
        };

        // Per-task evidence from the append-only stream (the summaries the
        // bridge wrote at commit time carry tokens/model/cost truth).
        let log = EventLog::open(
            &self.rundir().join("events.jsonl"),
            &self.cfg.spec_id,
            &self.cfg.run_id,
        )?;
        let events = log.replay()?;
        let escalations = events
            .iter()
            .filter(|e| e.event == Ev::EscalatedToCEO)
            .count() as u32;
        let retries: u32 = rows.iter().map(|(_, _, _, r)| *r as u32).sum();
        // Durations come from the LEDGER's started/finished stamps — the
        // driver's injected-clock domain — never from the event envelope's
        // wall-clock `ts`: identical final states must hash identically
        // (§21.2), and the report is derived input to ledger payload hashes
        // (the narrate turn), so wall-clock there would break determinism.
        let mut stamps: std::collections::BTreeMap<String, (Option<String>, Option<String>)> =
            std::collections::BTreeMap::new();
        {
            let mut stmt = conn.prepare(
                "SELECT task_id, started_at, finished_at FROM ledger WHERE spec_id = ?1 AND kind = 'agent_call'",
            )?;
            let it = stmt.query_map(rusqlite::params![self.cfg.spec_id], |r| {
                Ok((r.get::<_, Option<String>>(0)?, r.get(1)?, r.get(2)?))
            })?;
            for row in it {
                let (task, start, fin) = row?;
                if let Some(t) = task {
                    stamps.insert(t, (start, fin));
                }
            }
        }

        let mut tasks = Vec::new();
        for (id, status, agent, _) in rows {
            let mut fact = TaskFact {
                id: id.clone(),
                agent,
                status,
                ..Default::default()
            };
            for ev in events
                .iter()
                .filter(|e| e.task_id.as_deref() == Some(id.as_str()))
            {
                match ev.event {
                    Ev::TaskCompleted | Ev::TaskFailed => {
                        let summary = ev.data.get("summary").cloned().unwrap_or_default();
                        let s = |k: &str| {
                            summary
                                .get(k)
                                .and_then(|v| v.as_str())
                                .map(|x| x.to_string())
                        };
                        if ev.event == Ev::TaskCompleted {
                            fact.artifacts = summary
                                .get("artifacts")
                                .and_then(|a| a.as_array())
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                                        .collect()
                                })
                                .unwrap_or_default();
                            fact.model = s("model");
                            fact.provider = s("provider");
                            if let Some(t) = summary.get("tokens") {
                                fact.tokens_in = t.get("in").and_then(|v| v.as_u64()).unwrap_or(0);
                                fact.tokens_out =
                                    t.get("out").and_then(|v| v.as_u64()).unwrap_or(0);
                                fact.cached_tokens =
                                    t.get("cached").and_then(|v| v.as_u64()).unwrap_or(0);
                            }
                            fact.cost_usd = summary
                                .get("cost_usd")
                                .and_then(|v| v.as_f64())
                                .unwrap_or(0.0);
                            fact.cache_savings_usd = summary
                                .get("cache_savings_usd")
                                .and_then(|v| v.as_f64())
                                .unwrap_or(0.0);
                        } else {
                            fact.errors =
                                vec![s("summary").unwrap_or_else(|| "task failed".into())];
                        }
                    }
                    _ => {}
                }
            }
            fact.duration_seconds = match stamps.get(&id) {
                Some((Some(st), Some(fi))) => {
                    match (clock::epoch_from_utc_iso(st), clock::epoch_from_utc_iso(fi)) {
                        (Some(a), Some(b)) if b >= a => Some(b - a),
                        _ => None,
                    }
                }
                _ => None,
            };
            tasks.push(fact);
        }
        Ok(RunFacts {
            spec,
            plan,
            tasks,
            spent_usd: spent,
            budget_usd: budget,
            duration_seconds,
            retries,
            escalations,
            generated_at: clock::fmt_epoch_seconds(self.cfg.now_epoch),
        })
    }

    fn all_done(&self) -> Result<bool> {
        let store = StateStore::open(&self.cfg.root.join(".ribozym").join("state.db"))?;
        let n: i64 = store.conn().query_row(
            "SELECT COUNT(*) FROM tasks WHERE spec_id = ?1 AND status != 'done'",
            rusqlite::params![self.cfg.spec_id],
            |r| r.get(0),
        )?;
        Ok(n == 0)
    }
}

impl PlanningPipeline for AgentPipeline {
    fn save_form(&mut self, _o: &mut Orchestrator) -> Result<()> {
        let path = self.rundir().join("raw_input.md");
        if path.exists() {
            // A changed form under the same spec id is a provenance break —
            // the draft that follows must analyze the bytes it hash-names.
            let existing = std::fs::read(&path)?;
            if existing != self.form {
                return Err(Error::ConfigInvalid(format!(
                    "raw_input.md at {} exists with DIFFERENT bytes — a run's form is \
                     immutable; use a new spec id",
                    path.display()
                )));
            }
            return Ok(());
        }
        std::fs::create_dir_all(self.rundir())?;
        std::fs::write(&path, &self.form)?;
        let ts = clock::fmt_epoch_seconds(self.cfg.now_epoch);
        self.archive()
            .write_section("001-raw_input.md", &self.form, &ts)?;
        Ok(())
    }

    fn draft_spec(&mut self, o: &mut Orchestrator) -> Result<()> {
        let form = std::fs::read(self.rundir().join("raw_input.md"))?;
        let form_hash = hash::sha256_hex(&form);
        let guard = self.guard.clone();
        let form_path = format!("runs/{}/raw_input.md", self.cfg.spec_id);
        let payload = serde_json::json!({ "turn": "a1-draft", "form_sha256": form_hash });
        let mut doc = o.run_op("a1-draft", &payload, &mut || {
            let (spec, usage) =
                block_on_agent(A1Architect::draft_spec(&guard, &form, &form_path))??;
            Ok((serde_json::to_value(&spec)?, usage.cost_usd()))
        })?;
        // Identity is the CALLER's decision (the run directory is named for
        // it): a model inventing a different (valid-looking) id is stamped
        // back, loudly — same discipline as A2's plan re-stamping.
        if doc.get("id").and_then(serde_json::Value::as_str) != Some(self.cfg.spec_id.as_str()) {
            tracing::warn!(
                agent = "A1",
                claimed = doc
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
                "draft spec id did not match the run — stamped from the run identity"
            );
            doc["id"] = serde_json::json!(self.cfg.spec_id);
        }
        let doc_bytes = write_doc(&self.cfg.root, &self.cfg.spec_id, "spec", &doc)?;
        let ts = clock::fmt_epoch_seconds(self.cfg.now_epoch);
        self.archive()
            .append("spec.json", &std::fs::read(&doc_bytes)?, &ts)?;
        // The validated spec owns the run budget (§8.2: the config default
        // only pre-dates the spec).
        let spec: Spec = serde_json::from_value(doc.clone())
            .map_err(|e| Error::Agent(format!("drafted spec unreadable: {e}")))?;
        o.set_run_budget(spec.budget.max_cost_usd)?;
        Ok(())
    }

    fn lock_decisions(&mut self, _o: &mut Orchestrator) -> Result<bool> {
        let spec = self.spec()?;
        // §5.4C: the draft must carry locked ADR decisions, requirements and
        // a positive budget — anything less is an under-specified spec that
        // would force silent guesses downstream. Park for the user instead.
        Ok(!spec.requirements.is_empty()
            && !spec.decisions.is_empty()
            && spec.budget.max_cost_usd > 0.0)
    }

    fn plan_tasks(&mut self, o: &mut Orchestrator) -> Result<TaskPlan> {
        let spec = self.spec()?;
        let spec_hash =
            hash::sha256_hex(hash::canonical_json(&serde_json::to_value(&spec)?).as_bytes());
        let guard = self.guard.clone();
        let payload = serde_json::json!({ "turn": "a2-plan", "spec_sha256": spec_hash });
        let doc = o.run_op("a2-plan", &payload, &mut || {
            let (plan, usage) = block_on_agent(A2Ceo::make_plan(&guard, &spec))??;
            Ok((serde_json::to_value(&plan)?, usage.cost_usd()))
        })?;
        let doc_bytes = write_doc(&self.cfg.root, &self.cfg.spec_id, "plan", &doc)?;
        let ts = clock::fmt_epoch_seconds(self.cfg.now_epoch);
        self.archive()
            .append("plan.json", &std::fs::read(&doc_bytes)?, &ts)?;
        let plan: TaskPlan = serde_json::from_value(doc)
            .map_err(|e| Error::Agent(format!("planned tasks unreadable: {e}")))?;
        Ok(plan)
    }
}

impl FinalPipeline for AgentPipeline {
    fn finalize(&mut self, o: &mut Orchestrator) -> Result<()> {
        let spec = self.spec()?;
        let plan = self.plan()?;
        let facts = self.collect_facts(&spec, &plan)?;
        let skeleton = A3Reporter::skeleton(&facts);
        // Narrative turn is ledger-guarded too: the skeleton is deterministic
        // per final state, so a crash-resumed finalize reuses the saved story
        // instead of re-invoicing the model.
        let skeleton_hash =
            hash::sha256_hex(hash::canonical_json(&serde_json::to_value(&skeleton)?).as_bytes());
        let guard = self.guard.clone();
        let payload = serde_json::json!({ "turn": "a3-narrate", "skeleton_sha256": skeleton_hash });
        let story = o.run_op("a3-narrate", &payload, &mut || {
            match block_on_agent(A3Reporter::narrate(&guard, &skeleton))? {
                // Provider failure degrades to the numeric-only skeleton —
                // visibly (warn inside narrate), and honestly $0.
                Some((merged, usage)) => Ok((serde_json::to_value(&merged)?, usage.cost_usd())),
                None => Ok((serde_json::to_value(&skeleton)?, 0.0)),
            }
        })?;
        let with_story: DepartmentReport = serde_json::from_value(story)
            .map_err(|e| Error::Agent(format!("sealed report unreadable: {e}")))?;
        self.report = Some(with_story.clone());
        write_doc(
            &self.cfg.root,
            &self.cfg.spec_id,
            "report",
            &A3Reporter::to_canonical_json(&with_story)?,
        )?;
        Ok(())
    }

    fn review(&mut self, o: &mut Orchestrator) -> Result<bool> {
        // §5.7 m4: the sealed report exists BEFORE the verdict; on a resume
        // that re-enters FinalReview without a fresh finalize, reload from
        // disk — the run directory is the durable record.
        let mut report = match self.report.clone() {
            Some(r) => r,
            None => {
                let raw = std::fs::read(self.doc_path("report")).map_err(|e| {
                    Error::Agent(format!(
                        "FinalReview without a report (§5.7 m4 — finalize must run first): {e}"
                    ))
                })?;
                if !self
                    .archive()
                    .matches_any("report.json", &raw)
                    .map_err(|e| Error::ArchiveCorrupt(format!("report reload: {e}")))?
                {
                    return Err(Error::ArchiveCorrupt(
                        "report.json matches no sealed archive section".into(),
                    ));
                }
                serde_json::from_slice(&raw)?
            }
        };

        // The evidence block A1 referees against: CODE-assembled facts, not
        // the model's own earlier claims.
        let spec = self.spec()?;
        let plan = self.plan()?;
        let facts = self.collect_facts(&spec, &plan)?;
        let mut evidence = String::new();
        for t in &facts.tasks {
            evidence.push_str(&format!(
                "TASK {} agent={} status={} artifacts={} errors={}\n",
                t.id,
                t.agent,
                t.status,
                t.artifacts.join(","),
                t.errors.join(" | ")
            ));
        }
        // The verdict turn is ledger-guarded: identical evidence reuses the
        // saved verdict (a crisis re-review costs no new invoice and cannot
        // drift from the first one).
        let evidence_hash = hash::sha256_hex(evidence.as_bytes());
        let guard = self.guard.clone();
        let payload = serde_json::json!({ "turn": "a1-review", "evidence_sha256": evidence_hash });
        let cc: ComplianceCheck =
            serde_json::from_value(o.run_op("a1-review", &payload, &mut || {
                let (cc, usage) =
                    block_on_agent(A1Architect::final_review(&guard, &spec, &evidence))??;
                Ok((serde_json::to_value(&cc)?, usage.cost_usd()))
            })?)
            .map_err(|e| Error::Agent(format!("compliance verdict unreadable: {e}")))?;
        self.last_verdict = Some(cc.clone());
        A3Reporter::seal(&mut report, &cc);
        // P2 honesty, twice-locked: the sealed report must still agree with
        // the work it describes; disagreement blocks delivery.
        let all_done = self.all_done()?;
        let agree = A3Reporter::agrees_with_work(&report, all_done);
        if !agree {
            tracing::warn!(
                "sealed report disagrees with work truth (sub-100 or red tasks) — delivery blocked regardless of pct"
            );
        }
        // Persist the sealed report + the raw verdict as its own auditable
        // document (runs/<spec>/compliance.json).
        let cpath = write_doc(
            &self.cfg.root,
            &self.cfg.spec_id,
            "compliance",
            &serde_json::to_value(&cc)?,
        )?;
        let sealed_value = A3Reporter::to_canonical_json(&report)?;
        let rpath = write_doc(&self.cfg.root, &self.cfg.spec_id, "report", &sealed_value)?;
        // Seal the minutes (A12): the compliance verdict and the SEALED
        // report (post-review state — the pre-seal report is superseded here).
        let ts = clock::fmt_epoch_seconds(self.cfg.now_epoch);
        self.archive()
            .append("compliance.json", &std::fs::read(&cpath)?, &ts)?;
        self.archive()
            .append("report.json", &std::fs::read(&rpath)?, &ts)?;
        self.report = Some(report);
        // The report must be schema-valid on its way out — a corrupt merge is
        // caught here, not at the user's eyes.
        let problems = validate::validate_full(
            &self.schema_dir,
            "department_report.schema.json",
            &sealed_value,
        );
        if !problems.is_empty() {
            return Err(Error::Agent(format!(
                "sealed report fails its own schema: {}",
                problems.join("; ")
            )));
        }
        Ok(cc.delivery_ok() && agree)
    }

    /// §5.6 m5 (E2): the CEO re-cuts the unfinished work in scope. Guarded
    /// like every turn: op identity = (current plan hash, verdict hash,
    /// done set) so a resumed crisis never re-invoices a revision already
    /// written; the revised plan is absorbed (done rows untouched) and the
    /// run re-enters Scheduling via Trigger::Replanned.
    fn replan(&mut self, o: &mut Orchestrator) -> Result<ReplanOutcome> {
        let verdict = match self.last_verdict.clone() {
            Some(v) => v,
            None => return Ok(ReplanOutcome::NoChange),
        };
        let spec = self.spec()?;
        let current = self.plan()?;
        let done = o.done_task_ids();
        let why = format!(
            "unmet: {} ({}% match, verdict {})",
            verdict.unmet_requirements.join(","),
            verdict.spec_match_pct,
            verdict.verdict
        );
        let payload = serde_json::json!({
            "turn": "a2-replan",
            "plan_sha256": hash::sha256_hex(hash::canonical_json(&serde_json::to_value(&current)?).as_bytes()),
            "verdict_sha256": hash::sha256_hex(hash::canonical_json(&serde_json::to_value(&verdict)?).as_bytes()),
            "done": done,
        });
        let doc = o.run_op("a2-replan", &payload, &mut || {
            let (plan, usage) =
                block_on_agent(A2Ceo::replan(&self.guard, &spec, &current, &done, &why))??;
            Ok((serde_json::to_value(&plan)?, usage.cost_usd()))
        })?;
        let mut plan: TaskPlan = serde_json::from_value(doc)
            .map_err(|e| Error::Agent(format!("revised plan unreadable: {e}")))?;
        // The document mirrors the DB truth: completed rows stay `done` in
        // the revised plan too (plan.json is the CEO's INTENT, but a stale
        // "pending" over a finished row is a lie the audit trail must not
        // carry — §7.1).
        for t in &mut plan.tasks {
            if done.iter().any(|d| d == &t.id) {
                t.status = "done".into();
            }
        }
        let doc_bytes = write_doc(
            &self.cfg.root,
            &self.cfg.spec_id,
            "plan",
            &serde_json::to_value(&plan)?,
        )?;
        let ts = clock::fmt_epoch_seconds(self.cfg.now_epoch);
        self.archive()
            .append("plan.json", &std::fs::read(&doc_bytes)?, &ts)?;
        o.absorb_revised_plan(&plan)?;
        tracing::warn!(
            agent = "A2",
            revision = plan.revision,
            new = plan.tasks.len(),
            "E2 in-scope replan absorbed"
        );
        Ok(ReplanOutcome::Revised)
    }
}
