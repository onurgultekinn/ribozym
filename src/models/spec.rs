//! Spec (A1 output) — `schemas/spec.schema.json` (§11.1).

use serde::{Deserialize, Serialize};

use super::Extra;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Spec {
    pub schema_version: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub version: String,
    pub created_at: String,
    pub created_by: String,
    pub source: SourceRef,
    pub title: String,
    pub description: String,
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    pub language: Language,
    #[serde(default)]
    pub requirements: Vec<Requirement>,
    #[serde(default)]
    pub constraints: Vec<IdText>,
    #[serde(default)]
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    #[serde(default)]
    pub out_of_scope: Vec<String>,
    #[serde(default)]
    pub open_questions: Vec<OpenQuestion>,
    #[serde(default)]
    pub assumptions: Vec<Assumption>,
    #[serde(default)]
    pub decisions: Vec<Decision>,
    #[serde(default)]
    pub mvp_scope: Vec<String>,
    pub budget: Budget,
    pub autonomy_level: u8,
    #[serde(default)]
    pub approval_required: Vec<String>,
    pub compliance_status: String,
    #[serde(default)]
    pub compliance_notes: Vec<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
crate::impl_extra!(Spec);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceRef {
    pub raw_input_hash: String,
    pub raw_input_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Language {
    pub primary: String,
    pub confidence: f64,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Requirement {
    pub id: String,
    pub text: String,
    pub priority: String,
    pub category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IdText {
    pub id: String,
    pub text: String,
}

/// Question left open at spec time (silent-guess ban, v2.4).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenQuestion {
    pub id: String,
    pub question: String,
}

/// Explicit assumption; audited at FinalReview (v2.4).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Assumption {
    pub id: String,
    pub assumption: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AcceptanceCriterion {
    pub id: String,
    pub text: String,
    pub verifiable_by: String,
    pub measurement: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Decision {
    pub id: String,
    pub title: String,
    pub choice: String,
    #[serde(default)]
    pub alternatives_considered: Vec<Alternative>,
    pub rationale: String,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Alternative {
    pub option: String,
    pub rejected_because: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Budget {
    pub max_cost_usd: f64,
    pub max_duration_seconds: u64,
}
