//! Inference layer (§10). One provider trait, three implementations:
//! `cloud` (OpenRouter, primary engine — Q1), `local` (llama-server,
//! auxiliary offload), `mock` (tests only — §21.1: LLM is never called live
//! in tests).
//!
//! The provider returns raw text; schema validation of LLM output happens
//! ABOVE this layer (control plane, §3.1) — providers never self-approve.

pub mod cloud;
pub mod env;
pub mod local;
pub mod mock;

pub use cloud::CloudProvider;
pub use env::{EnvLayer, DEFAULT_BACKOFF_MS};
pub use local::LocalProvider;
pub use mock::{MockProvider, MockStep};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// One chat completion request. `system_prefix` is byte-stable across a run
/// so OpenRouter prompt caching triggers (§10.4 fixed-prefix rule).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceRequest {
    pub task_type: String,
    pub system_prefix: String,
    pub context: String,
    pub input: String,
    /// Maximum output tokens hint; providers clamp.
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

/// Provider-agnostic usage accounting; the provider `usage` field is the
/// single source of truth for token counts (§10.4).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    pub tokens_in: u64,
    pub tokens_out: u64,
    #[serde(default)]
    pub cached_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceResponse {
    pub text: String,
    pub usage: Usage,
    /// Which (provider, model) actually served the call.
    pub provider: String,
    pub model: String,
}

/// Honest per-turn accounting for the CONTROL-PLANE hats (A1/A2/A3): a
/// planning turn bills exactly like a task turn — provider-reported usage
/// carried out of the agent so the pipeline can charge it through the
/// ledger-guarded `run_op` (§8.2: management work is not a free lunch).
#[derive(Debug, Clone, PartialEq)]
pub struct TurnUsage {
    pub provider: String,
    pub model: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cached_tokens: u64,
    /// 1 = first-try pass, 2 = after the §14 repair turn.
    pub turns: u32,
}

impl TurnUsage {
    pub fn of(resp: &InferenceResponse, turns: u32) -> Self {
        Self {
            provider: resp.provider.clone(),
            model: resp.model.clone(),
            tokens_in: resp.usage.tokens_in,
            tokens_out: resp.usage.tokens_out,
            cached_tokens: resp.usage.cached_tokens,
            turns,
        }
    }

    /// §8.2 m1 price table (same tables the task path bills through).
    pub fn cost_usd(&self) -> f64 {
        estimate_cost(&self.provider, self.tokens_in, self.tokens_out)
    }
}

/// Provider price table — §8.2 m1. Public: A4's LLM turns price through it
/// too. models.toml seeding arrives with A10 (Faz 8); until then the two REAL
/// engines carry fixed published list prices so cost accounting is never
/// zero-by-ignorance.
pub fn price_of(provider: &str) -> PriceSpec {
    match provider {
        "openrouter" => PriceSpec {
            in_price_per_m: 0.20,
            out_price_per_m: 0.60,
        },
        "local" => PriceSpec {
            in_price_per_m: 0.0,
            out_price_per_m: 0.0,
        },
        _ => PriceSpec {
            in_price_per_m: 0.0,
            out_price_per_m: 0.0,
        },
    }
}

pub fn estimate_cost(provider: &str, tokens_in: u64, tokens_out: u64) -> f64 {
    price_of(provider).estimate(tokens_in, tokens_out)
}

/// Cost of a request in USD, computed by the cost guard BEFORE the call
/// (estimate) and reconciled AFTER (actual, from `Usage`).
#[derive(Debug, Clone, Copy)]
pub struct PriceSpec {
    pub in_price_per_m: f64,
    pub out_price_per_m: f64,
}

impl PriceSpec {
    pub fn estimate(&self, tokens_in: u64, tokens_out: u64) -> f64 {
        (tokens_in as f64 / 1_000_000.0) * self.in_price_per_m
            + (tokens_out as f64 / 1_000_000.0) * self.out_price_per_m
    }
}

/// Failures a provider can surface; the escalation ladder (§8.1) maps on these.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ProviderError {
    #[error("NETWORK_TIMEOUT")]
    Timeout,
    #[error("NETWORK_PROVIDER_429: retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },
    #[error("MODEL_UNAVAILABLE: {0}")]
    Unavailable(String),
    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },
    /// §8.2/B3: the call-boundary wall refused this call for money reasons.
    /// Distinct from Unavailable on purpose: a budget refusal is NOT
    /// transient — retrying it cannot succeed until the user raises the
    /// budget, so it must never enter the E0 backoff loop and the control
    /// plane pauses the run instead of failing the task.
    #[error("MODEL_BUDGET_DENIED: {0}")]
    Budget(String),
}

impl From<ProviderError> for crate::error::Error {
    fn from(e: ProviderError) -> Self {
        crate::error::Error::Provider(Box::new(e))
    }
}

#[async_trait]
pub trait InferenceProvider: Send + Sync {
    fn name(&self) -> &'static str;
    /// Serve one completion. Implementations must be idempotent-safe to retry:
    /// no side effects beyond the call itself.
    async fn complete(
        &self,
        request: &InferenceRequest,
    ) -> std::result::Result<InferenceResponse, ProviderError>;
}
