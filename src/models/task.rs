//! Task DAG (A2 output) — `schemas/tasks.schema.json` (§11.2, §6.2).

use serde::{Deserialize, Serialize};

use super::Extra;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TaskPlan {
    pub schema_version: String,
    pub spec_id: String,
    pub created_at: String,
    pub created_by: String,
    pub revision: u32,
    #[serde(default)]
    pub tasks: Vec<TaskItem>,
    #[serde(default)]
    pub execution_order: Vec<String>,
    #[serde(default)]
    pub parallel_groups: Vec<Vec<String>>,
    #[serde(default)]
    pub critical_path: Vec<String>,
    #[serde(default)]
    pub status_summary: StatusSummary,
    #[serde(flatten)]
    pub extra: Extra,
}
crate::impl_extra!(TaskPlan);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskItem {
    pub id: String,
    #[serde(default)]
    pub spec_requirement_ids: Vec<String>,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub department: Option<String>,
    pub assigned_agent: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolchain: Option<String>,
    pub acceptance: Acceptance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated: Option<Estimate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub difficulty: Option<Difficulty>,
    #[serde(default)]
    pub file_ownership: Vec<String>,
    #[serde(default)]
    pub artifacts_expected: Vec<String>,
    #[serde(default)]
    pub artifacts_produced: Vec<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    #[serde(default = "d_status")]
    pub status: String,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default)]
    pub escalation_level: u8,
    #[serde(default)]
    pub notes: String,
    #[serde(flatten)]
    pub extra: Extra,
}
crate::impl_extra!(TaskItem);

/// Deterministic estimate from `task_scores` history; `None` = no data, never
/// invented (v2.24 — "LLM tahmini yazmaz").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Estimate {
    #[serde(default)]
    pub tokens_in: u64,
    #[serde(default)]
    pub tokens_out: u64,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub duration_seconds: u64,
}

/// A10 difficulty analysis; `source` records the path taken: `rules` | `llm` (v2.44).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Difficulty {
    pub label: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Acceptance {
    pub verifiable_by: String,
    pub criteria: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct StatusSummary {
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub pending: u32,
    #[serde(default)]
    pub in_progress: u32,
    #[serde(default)]
    pub done: u32,
    #[serde(default)]
    pub failed: u32,
    #[serde(default)]
    pub skipped: u32,
}

fn d_status() -> String {
    "pending".into()
}
