//! Read-only surfaces (§20.1: `status`, `report`, `budget show`, `new-form`).
//! Everything here is DERIVED TRUTH — SQLite + sealed run documents, no LLM,
//! no side effects. A read surface that recomputed its own numbers would be
//! a second source of reality (§7.1); these functions only project what the
//! ledger, the task rows and the sealed report already say.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::params;

use crate::error::{Error, Result};
use crate::state::StateStore;

/// Per-run projection of the system of record.
#[derive(Debug, Clone, PartialEq)]
pub struct RunStatus {
    pub spec_id: String,
    pub run_id: String,
    pub phase: String,
    pub budget_usd: f64,
    pub spent_usd: f64,
    /// Task status → count, derived from the `tasks` table.
    pub tasks: BTreeMap<String, u32>,
    /// Gate awaiting a human vote, if any (`approvals` pending row).
    pub pending_gate: Option<String>,
    pub paused_reason: Option<String>,
    pub updated_at: String,
    /// §7.6 B2 sweep at read time: archive sections failing their manifest
    /// hash. Non-empty means the run's minutes are untrustworthy — surface
    /// it loudly instead of letting `status` look healthy.
    pub archive_corrupt: Vec<String>,
}

pub fn status_all(root: &Path) -> Result<Vec<RunStatus>> {
    let db = root.join(".ribozym").join("state.db");
    if !db.exists() {
        return Ok(Vec::new());
    }
    let store = StateStore::open(&db)?;
    let conn = store.conn();
    let mut stmt = conn.prepare(
        "SELECT spec_id, run_id, status, budget_usd, spent_usd, paused_reason, updated_at
         FROM runs ORDER BY spec_id",
    )?;
    type RunRow = (String, String, String, f64, f64, Option<String>, String);
    let rows: Vec<RunRow> = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::new();
    for (spec_id, run_id, phase, budget_usd, spent_usd, paused_reason, updated_at) in rows {
        let mut ts = BTreeMap::new();
        {
            let mut tstmt = conn
                .prepare("SELECT status, COUNT(*) FROM tasks WHERE spec_id = ?1 GROUP BY status")?;
            let it = tstmt.query_map(params![spec_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32))
            })?;
            for row in it {
                let (k, v) = row?;
                ts.insert(k, v);
            }
        }
        let pending_gate: Option<String> = conn
            .query_row(
                "SELECT gate FROM approvals WHERE spec_id = ?1 AND decision = 'pending'
                 ORDER BY ts LIMIT 1",
                params![spec_id],
                |r| r.get(0),
            )
            .ok();
        let corrupt = crate::archive::Archive::open(root, &spec_id).verify_all()?;
        out.push(RunStatus {
            spec_id,
            run_id,
            phase,
            budget_usd,
            spent_usd,
            tasks: ts,
            pending_gate,
            paused_reason,
            updated_at,
            archive_corrupt: corrupt,
        });
    }
    Ok(out)
}

/// The sealed report for a spec, read from the run directory. Errors when the
/// report has not been produced (never fabricated — §5.7).
pub fn report_json(root: &Path, spec_id: &str) -> Result<serde_json::Value> {
    let p = root.join("runs").join(spec_id).join("report.json");
    let raw = std::fs::read_to_string(&p).map_err(|e| {
        Error::ConfigInvalid(format!(
            "no sealed report at {}: A3 writes it at Finalizing (§5.7 m4) — {e}",
            p.display()
        ))
    })?;
    serde_json::from_str(&raw).map_err(|e| {
        Error::ConfigInvalid(format!("report.json for {spec_id} is not valid JSON: {e}"))
    })
}

/// Deterministic Markdown rendering of the sealed report — CODE only
/// (§5.7: the narrative was authored at seal time; this is a projector,
/// not an author).
pub fn render_md(report: &serde_json::Value) -> String {
    let g = |path: &str| -> String {
        // plain walk (fold on Option trips the Try-fold lint; the loop is
        // clearer anyway)
        let mut cur: Option<&serde_json::Value> = Some(report);
        for k in path.split('/') {
            cur = cur.and_then(|c| c.get(k));
        }
        cur.map(|v| match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_else(|| "—".into())
    };
    let num = |path: &str, default: &str| -> String {
        let v = g(path);
        if v == "—" {
            default.to_string()
        } else {
            v
        }
    };
    let mut md = String::new();
    md.push_str(&format!(
        "# Run report — {}\n\n",
        report["spec_id"].as_str().unwrap_or("?")
    ));
    md.push_str(&format!(
        "- status: **{}**\n",
        report["final_status"].as_str().unwrap_or("?")
    ));
    md.push_str(&format!(
        "- generated by: {} at {}\n",
        g("generated_by"),
        g("generated_at")
    ));
    if let Some(s) = report["executive_summary"].as_str() {
        md.push_str(&format!("\n> {s}\n"));
    }
    let sum = &report["summary"];
    md.push_str(&format!(
        "\n## Tasks\n\n| total | done | failed | skipped | retries | escalations |\n|---|---|---|---|---|---|\n\
         | {} | {} | {} | {} | {} | {} |\n",
        sum["tasks_total"], sum["tasks_done"], sum["tasks_failed"], sum["tasks_skipped"],
        sum["retries"], sum["escalations"]
    ));
    md.push_str(&format!(
        "\n## Cost\n\n- spent: ${} of ${} ({}% of budget)\n\
         - cloud: ${} · local: ${} · web: ${} · cache savings: ${}\n",
        num("cost/total_usd", "0"),
        num("cost/budget_limit_usd", "0"),
        num("cost/budget_used_pct", "0"),
        num("cost/breakdown/cloud_llm", "0"),
        num("cost/breakdown/local_inference", "0"),
        num("cost/breakdown/web_access", "0"),
        num("cost/cache_savings_usd", "0")
    ));
    let comp = &report["compliance"];
    if !comp.is_null() && comp["checked_by"] == "A1" {
        md.push_str(&format!(
            "\n## Compliance (A1)\n\n- spec_match: **{}%**\n- unmet: {}\n",
            num("compliance/spec_match_pct", "0"),
            comp["unmet_requirements"]
                .as_array()
                .map(|a| if a.is_empty() {
                    "none".to_string()
                } else {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_else(|| "—".into())
        ));
    }
    if let Some(trace) = report["traceability"].as_array() {
        if !trace.is_empty() {
            md.push_str("\n## Traceability\n\n| requirement | tasks | status | evidence |\n|---|---|---|---|\n");
            for t in trace {
                let tasks: Vec<&str> = t["task_ids"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
                    .unwrap_or_default();
                md.push_str(&format!(
                    "| {} | {} | {} | {} |\n",
                    t["requirement_id"].as_str().unwrap_or("?"),
                    tasks.join(", "),
                    t["status"].as_str().unwrap_or("?"),
                    t["evidence"].as_str().unwrap_or("—")
                ));
            }
        }
    }
    if let Some(tasks) = report["tasks"].as_array() {
        md.push_str("\n## Task log\n\n| id | agent | status | cost | model | artifacts |\n|---|---|---|---|---|---|\n");
        for t in tasks {
            let arts: Vec<&str> = t["artifacts"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
                .unwrap_or_default();
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                t["id"].as_str().unwrap_or("?"),
                t["agent"].as_str().unwrap_or("?"),
                t["status"].as_str().unwrap_or("?"),
                t["cost_usd"],
                t["model"].as_str().unwrap_or("—"),
                arts.join(" ")
            ));
        }
    }
    let warn = report["warnings"].as_array().cloned().unwrap_or_default();
    if !warn.is_empty() {
        md.push_str("\n## Warnings\n\n");
        for w in &warn {
            if let Some(s) = w.as_str() {
                md.push_str(&format!("- {s}\n"));
            }
        }
    }
    let next = report["next_steps"].as_array().cloned().unwrap_or_default();
    if !next.is_empty() {
        md.push_str("\n## Next steps\n\n");
        for n in &next {
            if let Some(s) = n.as_str() {
                md.push_str(&format!("- {s}\n"));
            }
        }
    }
    md
}

/// `budget show` projection: the configured frame + per-run truth.
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetSurface {
    pub monthly_usd: f64,
    pub soft_pct: f64,
    pub hard_pct: f64,
    pub runs: Vec<(String, String, f64, f64)>, // spec, status, spent, budget
    pub total_spent: f64,
}

pub fn budget_surface(
    root: &Path,
    monthly: f64,
    soft_pct: f64,
    hard_pct: f64,
) -> Result<BudgetSurface> {
    let runs = status_all(root)?;
    // fold, not sum(): std f64::Sum seeds with 0.0 - 0.0 == -0.0, and an
    // empty ledger would print as "-0.0000".
    let total_spent = runs.iter().fold(0.0f64, |acc, r| acc + r.spent_usd);
    Ok(BudgetSurface {
        monthly_usd: monthly,
        soft_pct,
        hard_pct,
        runs: runs
            .iter()
            .map(|r| {
                (
                    r.spec_id.clone(),
                    r.phase.clone(),
                    r.spent_usd,
                    r.budget_usd,
                )
            })
            .collect(),
        total_spent,
    })
}

/// The blank spec-form template (§5.4), embedded so the shipped binary
/// needs no data files. Never overwrites silently.
pub const FORM_TEMPLATE: &str = include_str!("../examples/spec_form.md");

pub fn new_form(out: &Path) -> Result<PathBuf> {
    if out.exists() {
        return Err(Error::ConfigInvalid(format!(
            "{} exists — refusing to overwrite a filled form (new-form is blank-only)",
            out.display()
        )));
    }
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok();
        }
    }
    std::fs::write(out, FORM_TEMPLATE)?;
    Ok(out.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn report_md_renders_every_truth_section() {
        // The fixture report exercises each projector branch.
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let doc =
            std::fs::read_to_string(root.join("examples/expected/report_valid.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&doc).unwrap();
        let md = render_md(&v);
        assert!(md.contains("# Run report — spec-20260912-001"));
        assert!(md.contains("**success**"));
        assert!(md.contains("| 2 | 2 | 0 | 0 |"));
        assert!(md.contains("spec_match: **100%**"));
        assert!(md.contains("R1"));
        assert!(md.contains("Project completed successfully"));
        assert!(md.contains("User may push to GitHub"));
    }

    #[test]
    fn report_md_survives_a_sparse_document() {
        // Required fields only — missing optional sections render empty,
        // never panic (a projector must not be able to crash on truth).
        let v = json!({
            "schema_version": "1.0",
            "spec_id": "spec-20260912-001",
            "generated_at": "2026-09-12T10:00:00Z",
            "generated_by": "A3",
            "final_status": "failed",
            "summary": {"tasks_total": 1, "tasks_done": 0, "tasks_failed": 1,
                        "tasks_skipped": 0, "retries": 0, "escalations": 0},
            "cost": {"total_usd": 0.0, "breakdown": {}, "budget_limit_usd": 0.0, "budget_used_pct": 0.0}
        });
        let md = render_md(&v);
        assert!(md.contains("**failed**"));
        assert!(!md.contains("Compliance"));
    }

    #[test]
    fn new_form_refuses_to_clobber_a_filled_form() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("spec_form.md");
        new_form(&p).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("## Budget"));
        let err = new_form(&p).unwrap_err();
        assert!(err.to_string().contains("refusing"), "{err}");
    }

    #[test]
    fn status_projection_reads_the_record_without_inventing_it() {
        // Empty world: no db, no runs — an empty honest list, not a fake row.
        let dir = tempfile::tempdir().unwrap();
        assert!(status_all(dir.path()).unwrap().is_empty());
        let b = budget_surface(dir.path(), 5.0, 80.0, 100.0).unwrap();
        assert_eq!(b.total_spent, 0.0);
        assert!(b.runs.is_empty());
    }
}
