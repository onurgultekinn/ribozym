//! `config/agents.toml` — agent registry (§11.7).
//! `model_tier` is a starting hint only; per-call selection belongs to A10 (v2.23).

use serde::Deserialize;

pub const VALID_TIERS: [&str; 4] = ["low", "medium", "high", "none"];

#[derive(Debug, Clone, Deserialize)]
pub struct AgentsFile {
    #[serde(default)]
    pub agent: Vec<AgentEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentEntry {
    pub id: String,
    pub title: String,
    pub prompt_file: String,
    pub model_tier: String,
    pub escalation_tier: String,
    #[serde(default)]
    pub max_concurrency: u32,
    #[serde(default)]
    pub budget_share_pct: u32,
    #[serde(default)]
    pub tools: Vec<String>,
}
