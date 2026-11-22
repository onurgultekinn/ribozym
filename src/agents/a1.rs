//! A1 — Chief Architect and Referee (§5.4/§5.4C, KİLİTLİ). Two hats, both
//! driven by the control plane (not DAG-dispatched):
//!
//! * **Architect** (`draft_spec`): the filled `spec_form.md` goes in as ONE
//!   big call (§5.4 stage 2); the model proposes the structured spec, the
//!   code pins PROVENANCE — `source.raw_input_hash` is computed over the
//!   exact form bytes and overwrites whatever the model claims. A model
//!   cannot manufacture evidence about input it did not see (§5.4C: silent
//!   guessing is forbidden, and so is silent forgery).
//! * **Referee** (`final_review`): the FinalReview verdict (§6.1). The model
//!   marks requirement statuses with evidence; the CODE recomputes
//!   `spec_match_pct` from those statuses, applies the §5.4C
//!   falsified-assumption penalty, rebuilds `unmet_requirements` and caps the
//!   verdict: delivery is only possible at 100 (P5) — an `approved` on a
//!   sub-100 score is downgraded, a stricter verdict is kept. This is the
//!   real counterpart of the former blanket `--assume-compliant`.

use crate::config;
use crate::error::{Error, Result};
use crate::hash;
use crate::models::compliance::{ComplianceCheck, RequirementVerdict};
use crate::models::spec::Spec;
use crate::repair::SchemaGuard;

/// §5.4C: "yanlış çıkan varsayım uyum yüzdesini düşürür" — the deterministic
/// penalty per falsified assumption (source-noted constant, not magic).
pub const FALSIFIED_ASSUMPTION_PENALTY_PCT: u8 = 10;

/// Weight of each requirement status in the recomputed match score
/// (partially met counts as half — conservative, never generous).
fn status_weight(status: &str) -> f64 {
    match status {
        "met" => 1.0,
        "partially_met" => 0.5,
        _ => 0.0,
    }
}

pub struct A1Architect;

impl A1Architect {
    pub fn new() -> Self {
        Self
    }

    fn system_prefix() -> String {
        config::agents_prompt_from(std::path::Path::new("."), "A1_architect")
            .unwrap_or_else(|_| "You are A1, the Chief Architect and Referee.".into())
    }

    /// Hat 1 — spec drafting (§5.4 stage 2). `form_bytes` must be the EXACT
    /// bytes of the form file; `form_path` the repo-relative path recorded in
    /// the spec's provenance block.
    pub async fn draft_spec(
        guard: &SchemaGuard,
        form_bytes: &[u8],
        form_path: &str,
    ) -> Result<(Spec, crate::inference::TurnUsage)> {
        let req = crate::inference::InferenceRequest {
            task_type: "spec".into(),
            system_prefix: Self::system_prefix(),
            context: String::new(),
            input: format!(
                "Analyze this filled spec form in one pass and emit the spec JSON. \
                 Anything uncertain becomes an open question or an explicit assumption — \
                 never a silent guess. Include ADR-format decisions[] for every technology \
                 choice (choice, alternatives_considered, rationale, evidence).\n\n\
                 FORM:\n{}",
                String::from_utf8_lossy(form_bytes)
            ),
            max_tokens: Some(8192),
        };
        let out = guard.complete(&req, "spec.schema.json").await?;
        let usage = crate::inference::TurnUsage::of(&out.response, out.turns);
        let mut spec: Spec = serde_json::from_value(out.value)
            .map_err(|e| Error::Agent(format!("spec document unreadable: {e}")))?;

        // Provenance is CODE's to write: hash of the exact input bytes, path
        // as given. The schema made the model produce *some* hash-shaped
        // string; only the code knows the real one.
        spec.schema_version = "1.0".into();
        spec.created_by = "A1".into();
        spec.source.raw_input_hash = format!("sha256:{}", hash::sha256_hex(form_bytes));
        spec.source.raw_input_path = form_path.to_string();
        Ok((spec, usage))
    }

    /// Hat 2 — FinalReview referee (§6.1). `evidence` is the code-assembled
    /// delivery block (task statuses + result summaries + artifact paths);
    /// the model only maps requirements onto it.
    pub async fn final_review(
        guard: &SchemaGuard,
        spec: &Spec,
        evidence: &str,
    ) -> Result<(ComplianceCheck, crate::inference::TurnUsage)> {
        let req = crate::inference::InferenceRequest {
            task_type: "verify".into(),
            system_prefix: Self::system_prefix(),
            context: String::new(),
            input: format!(
                "Referee the delivered work against this locked spec. For EVERY requirement \
                 id emit met/partially_met/unmet with concrete evidence from the delivery \
                 block. Audit EVERY assumption as held or falsified. Be strict: your claim \
                 is cross-checked and the score is recomputed from your statuses, never \
                 taken on faith; requirements or assumptions you do not review are counted \
                 against you.\n\n\
                 SPEC REQUIREMENTS:\n{}\n\n\
                 SPEC ASSUMPTIONS:\n{}\n\n\
                 SPEC DECISIONS (verify compliance where the evidence allows):\n{}\n\n\
                 DELIVERY EVIDENCE:\n{evidence}",
                serde_json::to_string_pretty(&spec.requirements)
                    .map_err(|e| Error::Agent(e.to_string()))?,
                serde_json::to_string_pretty(&spec.assumptions)
                    .map_err(|e| Error::Agent(e.to_string()))?,
                serde_json::to_string_pretty(&spec.decisions)
                    .map_err(|e| Error::Agent(e.to_string()))?,
            ),
            max_tokens: Some(4096),
        };
        let out = guard.complete(&req, "compliance_check.schema.json").await?;
        let mut cc: ComplianceCheck = serde_json::from_value(out.value)
            .map_err(|e| Error::Agent(format!("compliance document unreadable: {e}")))?;
        Self::adjudicate(&mut cc, spec);
        Ok((
            cc,
            crate::inference::TurnUsage::of(&out.response, out.turns),
        ))
    }

    /// The referee's code-side adjudication (pure, unit-testable): identity
    /// stamps, coverage completion, honest score, verdict cap.
    pub fn adjudicate(cc: &mut ComplianceCheck, spec: &Spec) {
        cc.schema_version = "1.0".into();
        cc.checked_by = "A1".into();
        cc.spec_id = spec.id.clone();

        // Fabricated verdicts are dropped BEFORE scoring: a model padding its
        // list with `{R99: met}` entries could otherwise inflate a real 0%
        // into a 50%. The denominator is the SPEC's requirement set only.
        let spec_ids: std::collections::BTreeSet<&str> =
            spec.requirements.iter().map(|r| r.id.as_str()).collect();
        let before = cc.requirements.len();
        cc.requirements.retain(|r| spec_ids.contains(r.id.as_str()));
        if cc.requirements.len() != before {
            tracing::warn!(
                agent = "A1",
                dropped = before - cc.requirements.len(),
                "compliance verdicts cited requirements the spec never declared"
            );
        }
        // A requirement the model skipped is UNMET, not absent — silence must
        // never read as success (§5.4C: silent guessing is forbidden both ways).
        for r in &spec.requirements {
            if !cc.requirements.iter().any(|v| v.id == r.id) {
                cc.requirements.push(RequirementVerdict {
                    id: r.id.clone(),
                    status: "unmet".into(),
                    evidence: "not reviewed by the model — treated as unmet".into(),
                    test_id: None,
                });
            }
        }

        // The score is OURS, not the model's: recomputed from the very
        // statuses it just emitted, minus the §5.4C assumption penalty.
        // Silence about an assumption is the dangerous direction (an
        // unaudited falsifiable claim could hide behind it), so an
        // unaudited assumption is completed as FALSIFIED — the referee is
        // told to audit every one, and this closes the loophole.
        let audited: std::collections::BTreeSet<String> = cc
            .assumptions_audit
            .iter()
            .map(|a| a.assumption_id.clone())
            .collect();
        let missing: Vec<String> = spec
            .assumptions
            .iter()
            .filter(|a| !audited.contains(&a.id))
            .map(|a| a.id.clone())
            .collect();
        for id in missing {
            cc.assumptions_audit
                .push(crate::models::compliance::AssumptionAudit {
                    assumption_id: id,
                    held: false,
                    note: Some("not audited by the model — counted as falsified (§5.4C)".into()),
                });
        }
        // Audit rows about assumptions the spec never declared cannot move
        // the score in either honest direction — dropped like fabricated
        // requirement verdicts (the same inflation attack, other list).
        let spec_assumptions: std::collections::BTreeSet<&str> =
            spec.assumptions.iter().map(|a| a.id.as_str()).collect();
        cc.assumptions_audit
            .retain(|a| spec_assumptions.contains(a.assumption_id.as_str()));

        let n = cc.requirements.len().max(1) as f64;
        let mut pct = (100.0
            * cc.requirements
                .iter()
                .map(|r| status_weight(&r.status))
                .sum::<f64>()
            / n)
            .floor() as i64;
        let falsified = cc.assumptions_audit.iter().filter(|a| !a.held).count() as i64;
        if falsified > 0 {
            pct = (pct - falsified * FALSIFIED_ASSUMPTION_PENALTY_PCT as i64).max(0);
        }
        cc.spec_match_pct = pct.clamp(0, 100) as u8;

        // Rebuild the unmet list from statuses — a model omitting its own
        // failures from the summary list cannot green-wash the delivery.
        cc.unmet_requirements = cc
            .requirements
            .iter()
            .filter(|r| r.status != "met")
            .map(|r| r.id.clone())
            .collect();

        // Verdict cap (P5): delivery only at 100. `approved` under 100 is a
        // downgrade; a model stricter than its own score keeps its word.
        if cc.spec_match_pct < 100 && cc.verdict == "approved" {
            cc.verdict = "needs_revision".into();
        }
    }
}

impl Default for A1Architect {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{MockProvider, MockStep};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn guard_with(steps: Vec<MockStep>) -> (SchemaGuard, Arc<MockProvider>) {
        let mock = Arc::new(MockProvider::new(steps));
        (SchemaGuard::new(mock.clone(), repo_root()), mock)
    }

    fn spec() -> Spec {
        let doc =
            std::fs::read_to_string(repo_root().join("examples/expected/spec_valid.json")).unwrap();
        serde_json::from_str(&doc).unwrap()
    }

    fn cc_json(spec_id: &str, pct: u8, verdict: &str, reqs: serde_json::Value) -> String {
        serde_json::json!({
            "schema_version": "1.0",
            "spec_id": spec_id,
            "checked_by": "A1",
            "spec_match_pct": pct,
            "requirements": reqs,
            "assumptions_audit": [{"assumption_id": "AS1", "held": true}],
            "verdict": verdict
        })
        .to_string()
    }

    // ---- hat 1: drafting --------------------------------------------------

    #[tokio::test]
    async fn provenance_hash_is_code_computed_even_when_the_model_lies() {
        let form = b"# Spec Form: tiny\n\n## Summary\nadd two numbers\n";
        // The model "claims" a hash it could not know.
        let body = serde_json::json!({
            "schema_version": "1.0",
            "id": "spec-20260912-001",
            "version": "1.0.0",
            "created_at": "2026-09-12T10:00:00Z",
            "created_by": "A1",
            "source": {
                "raw_input_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "raw_input_path": "somewhere/else.md"
            },
            "title": "tiny adder",
            "description": "adds",
            "mode": "greenfield",
            "language": {"primary": "rust", "confidence": 0.9, "source": "explicit"},
            "requirements": [
                {"id": "R1", "text": "add()", "priority": "must", "category": "functional"}
            ],
            "constraints": [],
            "acceptance_criteria": [],
            "out_of_scope": [],
            "budget": {"max_cost_usd": 1.0, "max_duration_seconds": 600},
            "autonomy_level": 1,
            "approval_required": [],
            "compliance_status": "approved"
        })
        .to_string();
        let (guard, _) = guard_with(vec![MockStep::Success {
            text: body,
            tokens_in: 400,
            tokens_out: 200,
        }]);
        let spec = A1Architect::draft_spec(&guard, form, "runs/x/spec_form.md")
            .await
            .unwrap()
            .0;
        assert_eq!(
            spec.source.raw_input_hash,
            format!("sha256:{}", hash::sha256_hex(form)),
            "model's hash claim must be overwritten with the real input hash"
        );
        assert_eq!(spec.source.raw_input_path, "runs/x/spec_form.md");
        assert_eq!(spec.created_by, "A1");
    }

    // ---- hat 2: refereeing -----------------------------------------------

    #[tokio::test]
    async fn claimed_pct_is_recomputed_from_statuses_not_trusted() {
        // The lying referee: claims 100 while marking R2 unmet. The stored
        // score is the code's (50), not the claim.
        let (guard, _) = guard_with(vec![MockStep::Success {
            text: cc_json(
                "spec-20260912-001",
                100,
                "approved",
                serde_json::json!([
                    {"id": "R1", "status": "met", "evidence": "src/routes.rs:12"},
                    {"id": "R2", "status": "unmet", "evidence": "absent"}
                ]),
            ),
            tokens_in: 400,
            tokens_out: 200,
        }]);
        let (cc, _) = A1Architect::final_review(&guard, &spec(), "evidence block")
            .await
            .unwrap();
        assert_eq!(cc.spec_match_pct, 50);
        assert_eq!(
            cc.verdict, "needs_revision",
            "P5: approved under 100 is downgraded"
        );
        assert_eq!(cc.unmet_requirements, vec!["R2".to_string()]);
        assert!(!cc.delivery_ok());
    }

    #[tokio::test]
    async fn unreviewed_requirement_counts_as_unmet() {
        // The model answered only R1; R2 must not vanish into silence.
        let (guard, _) = guard_with(vec![MockStep::Success {
            text: cc_json(
                "spec-20260912-001",
                100,
                "approved",
                serde_json::json!([
                    {"id": "R1", "status": "met", "evidence": "src/routes.rs:12"}
                ]),
            ),
            tokens_in: 400,
            tokens_out: 200,
        }]);
        let (cc, _) = A1Architect::final_review(&guard, &spec(), "evidence")
            .await
            .unwrap();
        assert_eq!(cc.requirements.len(), 2);
        let r2 = cc.requirements.iter().find(|r| r.id == "R2").unwrap();
        assert_eq!(r2.status, "unmet");
        assert_eq!(cc.spec_match_pct, 50);
    }

    #[tokio::test]
    async fn fabricated_requirement_cannot_inflate_the_score() {
        // The inflation attack: one real unmet + one REAL met + a made-up
        // "R99 met" would read 66% over the model's list. The denominator is
        // the spec's requirement set — fabrications are dropped first.
        let (guard, _) = guard_with(vec![MockStep::Success {
            text: cc_json(
                "spec-20260912-001",
                100,
                "approved",
                serde_json::json!([
                    {"id": "R1", "status": "unmet", "evidence": "absent"},
                    {"id": "R2", "status": "met", "evidence": "e2"},
                    {"id": "R99", "status": "met", "evidence": "invented"}
                ]),
            ),
            tokens_in: 400,
            tokens_out: 200,
        }]);
        let (cc, _) = A1Architect::final_review(&guard, &spec(), "evidence")
            .await
            .unwrap();
        assert_eq!(cc.requirements.len(), 2, "R99 never existed in the spec");
        assert_eq!(
            cc.spec_match_pct, 50,
            "computed over the REAL requirement set"
        );
    }

    #[tokio::test]
    async fn unaudited_assumption_counts_as_falsified() {
        // §5.4C's loophole: a perfect score WITH a silent assumption audit
        // must not sail through — silence is completed conservatively, so
        // the referee is forced to actually audit every assumption.
        let body = cc_json(
            "spec-20260912-001",
            100,
            "approved",
            serde_json::json!([
                {"id": "R1", "status": "met", "evidence": "e1"},
                {"id": "R2", "status": "met", "evidence": "e2"}
            ]),
        );
        let mut silent = serde_json::from_str::<serde_json::Value>(&body).unwrap();
        silent["assumptions_audit"] = serde_json::json!([]);
        let (guard, _) = guard_with(vec![MockStep::Success {
            text: silent.to_string(),
            tokens_in: 400,
            tokens_out: 200,
        }]);
        let (cc, _) = A1Architect::final_review(&guard, &spec(), "evidence")
            .await
            .unwrap();
        assert_eq!(cc.assumptions_audit.len(), 1);
        assert!(!cc.assumptions_audit[0].held);
        assert_eq!(cc.spec_match_pct, 90);
        assert!(!cc.delivery_ok(), "a silent audit cannot deliver");
    }

    #[tokio::test]
    async fn falsified_assumption_penalizes_the_score() {
        let body = cc_json(
            "spec-20260912-001",
            100,
            "approved",
            serde_json::json!([
                {"id": "R1", "status": "met", "evidence": "e1"},
                {"id": "R2", "status": "met", "evidence": "e2"}
            ]),
        );
        let mut with_audit = serde_json::from_str::<serde_json::Value>(&body).unwrap();
        with_audit["assumptions_audit"] = serde_json::json!([
            {"assumption_id": "AS1", "held": false, "note": "persistence was required after all"}
        ]);
        let (guard, _) = guard_with(vec![MockStep::Success {
            text: with_audit.to_string(),
            tokens_in: 400,
            tokens_out: 200,
        }]);
        let (cc, _) = A1Architect::final_review(&guard, &spec(), "evidence")
            .await
            .unwrap();
        assert_eq!(
            cc.spec_match_pct, 90,
            "one falsified assumption = -10 (§5.4C)"
        );
        assert_eq!(cc.verdict, "needs_revision");
    }

    #[tokio::test]
    async fn strict_model_verdict_survives_a_perfect_score() {
        // 100 with needs_revision: the model is stricter than its own score —
        // the code may not LOOSEN a verdict toward delivery.
        let (guard, _) = guard_with(vec![MockStep::Success {
            text: cc_json(
                "spec-20260912-001",
                100,
                "needs_revision",
                serde_json::json!([
                    {"id": "R1", "status": "met", "evidence": "e1"},
                    {"id": "R2", "status": "met", "evidence": "e2"}
                ]),
            ),
            tokens_in: 400,
            tokens_out: 200,
        }]);
        let (cc, _) = A1Architect::final_review(&guard, &spec(), "evidence")
            .await
            .unwrap();
        assert_eq!(cc.spec_match_pct, 100);
        assert_eq!(cc.verdict, "needs_revision");
        assert!(!cc.delivery_ok());
    }

    #[test]
    fn adjudicate_is_pure_and_stamps_identity() {
        let mut cc: ComplianceCheck = serde_json::from_value(serde_json::json!({
            "schema_version": "1.0",
            "spec_id": "spec-other",
            "checked_by": "A5",
            "spec_match_pct": 77,
            "requirements": [],
            "verdict": "approved"
        }))
        .unwrap();
        A1Architect::adjudicate(&mut cc, &spec());
        assert_eq!(cc.spec_id, "spec-20260912-001");
        assert_eq!(cc.checked_by, "A1");
        // both requirements missing → both unmet → 0
        assert_eq!(cc.spec_match_pct, 0);
        assert_eq!(cc.verdict, "needs_revision");
    }
}
