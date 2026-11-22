//! StandardDepartmentReport (A3 output, sealed) — `schemas/department_report.schema.json` (§11.5, §17).

use serde::{Deserialize, Serialize};

use super::Extra;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DepartmentReport {
    pub schema_version: String,
    pub spec_id: String,
    pub generated_at: String,
    pub generated_by: String,
    pub final_status: String,
    #[serde(default)]
    pub duration_seconds: u64,
    pub summary: ReportSummary,
    pub cost: Cost,
    #[serde(default)]
    pub quality_tradeoff: Vec<QualityTradeoff>,
    #[serde(default)]
    pub tasks: Vec<TaskRecord>,
    #[serde(default)]
    pub artifacts: ArtifactGroups,
    #[serde(default)]
    pub traceability: Vec<TraceEntry>,
    #[serde(default)]
    pub compliance: Compliance,
    #[serde(default)]
    pub idea_audit: Vec<IdeaAuditEntry>,
    #[serde(default)]
    pub health_notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executive_summary: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub next_steps: Vec<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
crate::impl_extra!(DepartmentReport);

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ReportSummary {
    #[serde(default)]
    pub tasks_total: u32,
    #[serde(default)]
    pub tasks_done: u32,
    #[serde(default)]
    pub tasks_failed: u32,
    #[serde(default)]
    pub tasks_skipped: u32,
    #[serde(default)]
    pub retries: u32,
    #[serde(default)]
    pub escalations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Cost {
    pub total_usd: f64,
    #[serde(default)]
    pub breakdown: CostBreakdown,
    #[serde(default)]
    pub cache_savings_usd: f64,
    #[serde(default)]
    pub budget_limit_usd: f64,
    #[serde(default)]
    pub budget_used_pct: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CostBreakdown {
    #[serde(default)]
    pub local_inference: f64,
    #[serde(default)]
    pub cloud_llm: f64,
    #[serde(default)]
    pub web_access: f64,
}

/// v2.47 B3: price-leaning model demotions under budget pressure; silent
/// quality loss is forbidden.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QualityTradeoff {
    pub task_id: String,
    pub model: String,
    pub tier_demoted_from: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskRecord {
    pub id: String,
    pub agent: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Tokens>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Tokens {
    #[serde(default, rename = "in")]
    pub in_tokens: u64,
    #[serde(default, rename = "out")]
    pub out_tokens: u64,
    #[serde(default, rename = "cached")]
    pub cached_tokens: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ArtifactGroups {
    #[serde(default)]
    pub source_files: Vec<String>,
    #[serde(default)]
    pub tests: Vec<String>,
    #[serde(default)]
    pub docs: Vec<String>,
    #[serde(default)]
    pub devops: Vec<String>,
}

/// Requirement -> task -> evidence map (§6.4 traceability).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceEntry {
    pub requirement_id: String,
    pub task_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_id: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Compliance {
    #[serde(default)]
    pub checked_by: String,
    #[serde(default)]
    pub spec_match_pct: u8,
    #[serde(default)]
    pub requirements: Vec<RequirementStatus>,
    #[serde(default)]
    pub unmet_requirements: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria_passed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RequirementStatus {
    pub id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

/// User suggestion loop audit (v2.7).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IdeaAuditEntry {
    pub idea_id: String,
    pub proposal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impact_summary: Option<String>,
    pub decision: String,
}
