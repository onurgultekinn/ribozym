//! `ribozym.toml` — project-level configuration (§18, §8.2, v2.47 values).
//! Values here are the knobs the doc references; nothing is duplicated in code.

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProjectConfig {
    #[serde(default)]
    pub run: RunSection,
    #[serde(default)]
    pub budget: BudgetSection,
    #[serde(default)]
    pub research: ResearchSection,
    #[serde(default)]
    pub a10_selection: A10Section,
    #[serde(default)]
    pub archive: ArchiveSection,
    #[serde(default)]
    pub web: WebSection,
    #[serde(default)]
    pub errors: ErrorsSection,
    #[serde(default)]
    pub sandbox: SandboxSection,
    /// §12.3 — per-toolchain allowlist (BTreeMap: stable order for hashing).
    #[serde(default)]
    pub toolchains: BTreeMap<String, ToolchainSpec>,
}

/// One `[toolchains.<name>]` entry (§12.3). `allowed` are space-separated
/// argv prefixes matched as FULL argument vectors (§12.2 — not a prefix match:
/// `cargo check --config build.rs=...` is rejected even though `cargo check`
/// is allowed).
#[derive(Debug, Clone, Deserialize)]
pub struct ToolchainSpec {
    #[serde(default)]
    pub allowed: Vec<String>,
    #[serde(default)]
    pub fs_read: Vec<String>,
    #[serde(default)]
    pub fs_write: Vec<String>,
    /// "deny" -> strip *_KEY/*_TOKEN/*_SECRET from the child env (§12.3).
    #[serde(default = "d_scrub_deny")]
    pub env_scrub: String,
    #[serde(default = "d_timeout")]
    pub timeout_s: u64,
    #[serde(default = "d_cap")]
    pub output_cap_kb: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RunSection {
    #[serde(default = "d_max_concurrency")]
    pub max_concurrency: u32,
    #[serde(default = "d_gate_timeout")]
    pub gate_timeout_seconds: u64,
    #[serde(default = "d_autonomy")]
    pub default_autonomy_level: u8,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct BudgetSection {
    #[serde(default)]
    pub monthly_usd: f64,
    #[serde(default = "d_test_quota")]
    pub test_quota_pct: f64,
    #[serde(default = "d_replan_pct")]
    pub replan_autonomous_cost_pct: f64,
    #[serde(default = "d_soft")]
    pub soft_limit_pct: f64,
    #[serde(default = "d_hard")]
    pub hard_limit_pct: f64,
    #[serde(default = "d_stale_margin")]
    pub price_stale_margin_pct: f64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ResearchSection {
    #[serde(default = "d_research_quota")]
    pub research_quota_pct: f64,
    #[serde(default = "d_one")]
    pub max_concurrent_experiments: u32,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct A10Section {
    #[serde(default = "d_price_poll")]
    pub price_poll_interval_s: u64,
    #[serde(default = "d_daily")]
    pub quality_score_refresh_hours: u64,
    #[serde(default = "d_window")]
    pub bench_history_window: u32,
    #[serde(default = "d_blacklist")]
    pub blacklist_consecutive_failures: u32,
    #[serde(default = "d_verify_min")]
    pub verify_min_confidence: u32,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ArchiveSection {
    #[serde(default = "d_compression")]
    pub compression: String,
    #[serde(default = "d_zstd_level")]
    pub level: u8,
    #[serde(default = "d_integrity")]
    pub integrity: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WebSection {
    #[serde(default = "d_fetcher")]
    pub fetcher: String,
    /// When fetcher = "command": the argv template (§13.1 agent-reach
    /// backend). A literal "{ask}" element receives the query text — argv,
    /// never a shell string.
    #[serde(default)]
    pub fetch_argv: Vec<String>,
    #[serde(default = "d_fallback")]
    pub fallback: String,
    #[serde(default = "d_ttl")]
    pub cache_ttl_s: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ErrorsSection {
    #[serde(default = "d_fix_loop")]
    pub fix_loop_max_iterations: u32,
    #[serde(default = "d_e0_retries")]
    pub e0_max_retries: u32,
    #[serde(default)]
    pub e0_backoff_ms: Vec<u64>,
    /// v2.47 B1: FinalReview -> Crisis loop cap.
    #[serde(default = "d_crisis_max")]
    pub final_review_crisis_max: u32,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SandboxSection {
    #[serde(default = "d_backend")]
    pub backend: String,
    #[serde(default = "d_network")]
    pub network: String,
}

fn d_max_concurrency() -> u32 {
    2
}
fn d_gate_timeout() -> u64 {
    1200
}
fn d_autonomy() -> u8 {
    1
}
fn d_test_quota() -> f64 {
    10.0
}
fn d_replan_pct() -> f64 {
    10.0
}
fn d_soft() -> f64 {
    80.0
}
fn d_hard() -> f64 {
    100.0
}
fn d_stale_margin() -> f64 {
    15.0
}
fn d_research_quota() -> f64 {
    5.0
}
fn d_one() -> u32 {
    1
}
fn d_price_poll() -> u64 {
    300
}
fn d_daily() -> u64 {
    24
}
fn d_window() -> u32 {
    20
}
fn d_blacklist() -> u32 {
    3
}
fn d_verify_min() -> u32 {
    50
}
fn d_compression() -> String {
    "zstd".into()
}
fn d_zstd_level() -> u8 {
    3
}
fn d_integrity() -> String {
    "sha256-manifest".into()
}
fn d_fetcher() -> String {
    "agent-reach".into()
}
fn d_fallback() -> String {
    "duckduckgo-html".into()
}
fn d_ttl() -> u64 {
    86_400
}
fn d_fix_loop() -> u32 {
    3
}
fn d_e0_retries() -> u32 {
    3
}
fn d_crisis_max() -> u32 {
    2
}
fn d_backend() -> String {
    "docker".into()
}
fn d_network() -> String {
    "none".into()
}
fn d_scrub_deny() -> String {
    "deny".into()
}
fn d_timeout() -> u64 {
    300
}
fn d_cap() -> u64 {
    1024
}
