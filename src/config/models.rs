//! `config/models.toml` — model registry (§11.6).
//! Seeding only: the live pool is filled by `ribozym calibrate` and run data
//! in `task_scores`; unmeasured models cannot be candidates (§10.3).

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ModelsFile {
    #[serde(default)]
    pub openrouter: OpenRouterSection,
    #[serde(default)]
    pub artificial_analysis: ArtificialAnalysisSection,
    #[serde(default)]
    pub selection: SelectionSection,
    #[serde(default)]
    pub local: LocalSection,
    #[serde(default, rename = "model")]
    pub models: Vec<ModelEntry>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ArtificialAnalysisSection {
    #[serde(default)]
    pub api_key_env: String,
    /// Quality-score refresh cadence (hours) for the A10 tier list.
    #[serde(default = "d_bench_ttl")]
    pub refresh_hours: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct OpenRouterSection {
    #[serde(default)]
    pub api_key_env: String,
    #[serde(default)]
    pub base_url: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SelectionSection {
    #[serde(default = "d_ceiling")]
    pub price_ceiling_multiplier: f64,
    #[serde(default = "d_min_quality")]
    pub min_quality_score: u32,
    #[serde(default = "d_price_ttl")]
    pub cache_price_ttl_minutes: u64,
    #[serde(default = "d_bench_ttl")]
    pub cache_benchmark_ttl_hours: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LocalSection {
    #[serde(default)]
    pub server_url_env: String,
    #[serde(default)]
    pub default_server_url: String,
    #[serde(default)]
    pub model_path: String,
    #[serde(default)]
    pub model_id: String,
    #[serde(default)]
    pub vram_budget_gb: u32,
    /// Q1/§10.1: the ONLY task types local may serve (grunt work). Empty list
    /// disables local entirely; routing never guesses beyond this whitelist.
    #[serde(default = "d_local_eligible")]
    pub eligible_task_types: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    #[serde(default)]
    pub task_types: Vec<String>,
    #[serde(default)]
    pub context_limit: u64,
    #[serde(default)]
    pub capability: String,
    #[serde(default)]
    pub cost_tier: String,
    #[serde(default)]
    pub blacklisted: bool,
    #[serde(default)]
    pub blacklist_reason: Option<String>,
}

fn d_ceiling() -> f64 {
    3.0
}
fn d_min_quality() -> u32 {
    60
}
fn d_price_ttl() -> u64 {
    5
}
fn d_bench_ttl() -> u64 {
    24
}

/// Plan §10.1 (Q1): local is only for cheap mechanical turns — template code,
/// simple test skeletons, classification; plus schema repair and summaries
/// (the 7B coder's reliable lanes). Everything else is cloud.
fn d_local_eligible() -> Vec<String> {
    [
        "boilerplate",
        "template_code",
        "test_skeleton",
        "classification",
        "schema_repair",
        "summarization",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}
