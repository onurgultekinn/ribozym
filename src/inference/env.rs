//! Env layer — the ONE wrapper every LLM call passes through (§3.1, §8.1 E0,
//! §8.2 pre-clamp, P4/P6). Retries transient errors with full-jitter backoff,
//! reconciles estimated vs actual cost, and refuses calls the budget cannot
//! cover. The router/selector (Phase 4, A10) sits on top of this.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use crate::error::{Error, Result};
use crate::inference::{
    InferenceProvider, InferenceRequest, InferenceResponse, PriceSpec, ProviderError,
};

/// E0 backoff schedule (§8.1: 5s -> 15s -> 45s full jitter). Tests shrink it.
pub const DEFAULT_BACKOFF_MS: [u64; 3] = [5000, 15000, 45000];

pub struct EnvLayer {
    provider: Arc<dyn InferenceProvider>,
    price: PriceSpec,
    backoff_ms: [u64; 3],
    max_retries: u32,
    /// Remaining budget this layer may spend (pre-clamp source, §8.2).
    remaining_budget: Arc<AtomicU64>, // in micro-USD to keep integers
    /// Deterministic RNG seed for jitter (test reproducibility).
    jitter_seed: AtomicU64,
}

/// Outcome of one guarded call, including the cost reconciliation the ledger
/// needs (§8.2: estimate BEFORE, actual AFTER).
#[derive(Debug)]
pub struct GuardedCall {
    pub response: InferenceResponse,
    pub estimated_cost_usd: f64,
    pub actual_cost_usd: f64,
    pub attempts: u32,
}

impl EnvLayer {
    pub fn new(
        provider: Arc<dyn InferenceProvider>,
        price: PriceSpec,
        remaining_budget_usd: f64,
    ) -> Self {
        Self::with_tuning(provider, price, remaining_budget_usd, 3, DEFAULT_BACKOFF_MS)
    }

    pub fn with_tuning(
        provider: Arc<dyn InferenceProvider>,
        price: PriceSpec,
        remaining_budget_usd: f64,
        max_retries: u32,
        backoff_ms: [u64; 3],
    ) -> Self {
        Self {
            provider,
            price,
            backoff_ms,
            max_retries,
            remaining_budget: Arc::new(AtomicU64::new((remaining_budget_usd * 1_000_000.0) as u64)),
            jitter_seed: AtomicU64::new(0x9E3779B97F4A7C15),
        }
    }

    fn next_jitter_ms(&self, ceiling_ms: u64) -> u64 {
        // xorshift64* — deterministic, no external rng dependency.
        let mut x = self.jitter_seed.load(Ordering::Relaxed);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.jitter_seed.store(x, Ordering::Relaxed);
        (x.wrapping_mul(0x2545F4914F6CDD1D) >> 33) % ceiling_ms.max(1)
    }

    /// Estimated cost of a call BEFORE it happens (§8.2 pre-clamp).
    pub fn estimate_cost(&self, tokens_in: u64, tokens_out: u64) -> f64 {
        self.price.estimate(tokens_in, tokens_out)
    }

    pub fn remaining_budget_usd(&self) -> f64 {
        self.remaining_budget.load(Ordering::Relaxed) as f64 / 1_000_000.0
    }

    /// One guarded completion: pre-clamp -> (retry w/ backoff) -> reconcile.
    /// Retries ONLY transient errors (E0: timeout, 429, 5xx-ish Unavailable);
    /// Http 4xx and budget refusal fail immediately.
    pub async fn complete_guarded(
        &self,
        request: &InferenceRequest,
        est_tokens_in: u64,
        est_tokens_out: u64,
    ) -> Result<GuardedCall> {
        let estimated = self.estimate_cost(est_tokens_in, est_tokens_out);

        // Pre-clamp: refuse what the budget cannot cover (§8.2 m3).
        if estimated > self.remaining_budget_usd() {
            return Err(Error::ModelBudgetDenied(format!(
                "estimated ${estimated:.6} exceeds remaining ${:.6}",
                self.remaining_budget_usd()
            )));
        }

        let mut attempts = 0u32;
        loop {
            attempts += 1;
            match self.provider.complete(request).await {
                Ok(resp) => {
                    let actual = self
                        .price
                        .estimate(resp.usage.tokens_in, resp.usage.tokens_out);
                    // Reconcile: CAS loop so concurrent guarded calls can never
                    // resurrect budget via a stale read between load and sub —
                    // deduct actual, saturating at zero.
                    let micro = (actual * 1_000_000.0) as u64;
                    let mut cur = self.remaining_budget.load(Ordering::Relaxed);
                    loop {
                        let next = cur.saturating_sub(micro);
                        match self.remaining_budget.compare_exchange_weak(
                            cur,
                            next,
                            Ordering::Relaxed,
                            Ordering::Relaxed,
                        ) {
                            Ok(_) => break,
                            Err(actual_cur) => cur = actual_cur,
                        }
                    }
                    return Ok(GuardedCall {
                        response: resp,
                        estimated_cost_usd: estimated,
                        actual_cost_usd: actual,
                        attempts,
                    });
                }
                Err(ProviderError::RateLimited { retry_after_ms }) => {
                    if attempts > self.max_retries {
                        return Err(Error::EnvLayerExhausted(format!(
                            "429 after {attempts} attempts"
                        )));
                    }
                    tracing::warn!(
                        attempts,
                        retry_after_ms,
                        "provider rate-limited; backing off"
                    );
                    let ceiling = if retry_after_ms > 0 {
                        retry_after_ms
                    } else {
                        self.backoff_ms[((attempts - 1) as usize).min(2)]
                    };
                    tokio::time::sleep(std::time::Duration::from_millis(
                        self.next_jitter_ms(ceiling).max(1),
                    ))
                    .await;
                }
                Err(ProviderError::Timeout) => {
                    if attempts > self.max_retries {
                        return Err(Error::EnvLayerExhausted(format!(
                            "timeout after {attempts} attempts"
                        )));
                    }
                    tracing::warn!(attempts, "provider timed out; backing off");
                    let ceiling = self.backoff_ms[((attempts - 1) as usize).min(2)];
                    tokio::time::sleep(std::time::Duration::from_millis(
                        self.next_jitter_ms(ceiling).max(1),
                    ))
                    .await;
                }
                Err(ProviderError::Unavailable(msg)) => {
                    if attempts > self.max_retries {
                        return Err(Error::EnvLayerExhausted(format!(
                            "unavailable after {attempts} attempts: {msg}"
                        )));
                    }
                    tracing::warn!(attempts, msg = %msg, "model unavailable; backing off (§8.1 E0)");
                    let ceiling = self.backoff_ms[((attempts - 1) as usize).min(2)];
                    tokio::time::sleep(std::time::Duration::from_millis(
                        self.next_jitter_ms(ceiling).max(1),
                    ))
                    .await;
                }
                Err(ProviderError::Budget(msg)) => {
                    // Never re-sleep on a money refusal (§8.2): the wall only
                    // moves when the user raises the budget.
                    return Err(Error::ModelBudgetDenied(msg));
                }
                Err(e @ ProviderError::Http { .. }) => {
                    // Non-transient: surface immediately, no retry (fail-fast, §8).
                    return Err(Error::Provider(Box::new(e)));
                }
            }
        }
    }
}

#[async_trait]
impl InferenceProvider for EnvLayer {
    fn name(&self) -> &'static str {
        self.provider.name()
    }

    /// Trait form for callers that already know the estimates; uses the
    /// request's own field sizes as a coarse estimate.
    async fn complete(
        &self,
        request: &InferenceRequest,
    ) -> std::result::Result<InferenceResponse, ProviderError> {
        let est_in = request.system_prefix.len() as u64
            + request.context.len() as u64
            + request.input.len() as u64;
        let est_out = request.max_tokens.unwrap_or(1024) as u64;
        let guarded = self
            .complete_guarded(request, est_in / 4, est_out) // ~4 chars/token heuristic
            .await
            .map_err(|e| match e {
                Error::Provider(p) => *p,
                // Budget denial is not transient: it surfaces as a dedicated
                // non-retryable error so the control plane pauses the run
                // (§8.2/B3) instead of burning backoff on a refused call.
                Error::ModelBudgetDenied(m) => ProviderError::Budget(m),
                other => ProviderError::Unavailable(other.to_string()),
            })?;
        Ok(guarded.response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::mock::{MockProvider, MockStep};

    fn req() -> InferenceRequest {
        InferenceRequest {
            task_type: "plan".into(),
            system_prefix: "s".into(),
            context: String::new(),
            input: "i".into(),
            max_tokens: Some(8_000),
        }
    }

    /// §8.2/B3: a money refusal must NOT masquerade as "model unavailable"
    /// (that would put the call into E0 backoff — a refused call can only
    /// succeed after the USER raises the wall).
    #[tokio::test]
    async fn budget_refusal_is_a_dedicated_non_retryable_error() {
        let mock = Arc::new(MockProvider::new(vec![MockStep::Success {
            text: "x".into(),
            tokens_in: 10,
            tokens_out: 10,
        }]));
        let layer = EnvLayer::with_tuning(
            mock.clone(),
            PriceSpec {
                in_price_per_m: 0.20,
                // large enough that the max_tokens estimate exceeds the wall
                out_price_per_m: 600.0,
            },
            0.000001,
            3,
            [1, 1, 1],
        );
        let err = layer
            .complete(&req())
            .await
            .expect_err("pre-clamp must refuse");
        assert!(
            matches!(err, ProviderError::Budget(_)),
            "budget refusal needs its own variant: {err:?}"
        );
        // The provider behind the wall was never touched — no wasted call.
        assert!(mock.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn unpriced_traffic_pauses_never_on_zero_estimate_wall() {
        // Free engine ($0 price): the wall can never deny; spending stays 0.
        let mock = Arc::new(MockProvider::always("hello"));
        let layer = EnvLayer::new(
            mock.clone(),
            PriceSpec {
                in_price_per_m: 0.0,
                out_price_per_m: 0.0,
            },
            0.000001,
        );
        let r = layer.complete(&req()).await.unwrap();
        assert_eq!(r.text, "hello");
    }
}
