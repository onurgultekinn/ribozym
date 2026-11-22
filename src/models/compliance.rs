//! ComplianceCheck (A1 final-review output) — `schemas/compliance_check.schema.json`
//! (§11.4, §6.1 FinalReview). The typed mirror of A1's referee verdict; the
//! control plane reads `spec_match_pct`/`verdict` but NEVER trusts them as
//! written — `agents::a1` recomputes the numbers from the requirement statuses
//! (§5.4C: a falsified assumption and an unmet requirement lower the score,
//! no matter what the model claims).

use serde::{Deserialize, Serialize};

use super::Extra;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComplianceCheck {
    pub schema_version: String,
    pub spec_id: String,
    pub checked_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub spec_match_pct: u8,
    #[serde(default)]
    pub requirements: Vec<RequirementVerdict>,
    #[serde(default)]
    pub decisions_compliance: Vec<DecisionCompliance>,
    #[serde(default)]
    pub assumptions_audit: Vec<AssumptionAudit>,
    #[serde(default)]
    pub unmet_requirements: Vec<String>,
    pub verdict: String,
    #[serde(flatten)]
    pub extra: Extra,
}
crate::impl_extra!(ComplianceCheck);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RequirementVerdict {
    pub id: String,
    pub status: String,
    pub evidence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionCompliance {
    pub decision_id: String,
    pub title: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssumptionAudit {
    pub assumption_id: String,
    pub held: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl ComplianceCheck {
    /// §6.1/P5: delivery is possible only at 100 with every requirement met
    /// and an `approved` verdict. The authoritative predicate — callers must
    /// use THIS, not the raw pct (the number alone can lie while verdicts
    /// disagree).
    pub fn delivery_ok(&self) -> bool {
        self.verdict == "approved"
            && self.spec_match_pct == 100
            && self.requirements.iter().all(|r| r.status == "met")
    }
}
