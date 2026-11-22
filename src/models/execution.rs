//! ExecutionResult (agent output contract) — `schemas/execution_result.schema.json` (§11.3).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Extra;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionResult {
    pub schema_version: String,
    pub task_id: String,
    pub agent_id: String,
    pub started_at: String,
    pub finished_at: String,
    pub status: String,
    pub summary: String,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub logs: Vec<LogEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_usage: Option<ModelUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolchain_used: Option<String>,
    #[serde(default)]
    pub commands_run: Vec<CommandRun>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
    #[serde(default)]
    pub escalate_to_ceo: bool,
    #[serde(default)]
    pub human_approval_needed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency: Option<Idempotency>,
    #[serde(flatten)]
    pub extra: Extra,
}
crate::impl_extra!(ExecutionResult);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Artifact {
    #[serde(rename = "type")]
    pub kind: String,
    pub path: String,
    pub hash: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LogEntry {
    pub level: String,
    pub message: String,
    pub ts: String,
}

/// `None` for deterministic agents (A7; A9/A10 template paths) — §11.3.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelUsage {
    pub provider: String,
    pub model: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    #[serde(default)]
    pub cached_tokens: u64,
    pub cost_usd: f64,
    #[serde(default)]
    pub cache_savings_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommandRun {
    pub cmd: String,
    pub exit_code: i32,
    pub duration_ms: u64,
}

/// Resume support: link into the operation ledger (§7.4).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Idempotency {
    pub op_id: String,
    pub input_hash: String,
}
