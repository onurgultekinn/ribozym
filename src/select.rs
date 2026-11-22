//! Grunt-work detection + local/cloud routing (Q1 decision, §10.1).
//!
//! Cloud is the primary engine; the local Qwen2.5-Coder-7B (llama-server) is
//! an auxiliary cost/latency accelerator used ONLY for cheap task types the
//! config marks eligible (§10.1: "şablon kod, basit test iskeleti,
//! sınıflandırma"). Two hard rules from the plan are implemented here:
//!
//! * **VRAM admission (§10.1):** if requested context + model load exceeds
//!   free VRAM, the task silently goes to cloud — no failure, no waiting.
//! * **Silent fallback:** any local provider error (server down, timeout)
//!   falls through to cloud without escalation semantics; the response's
//!   `provider` field records which engine actually served the call.
//!
//! Classification is rule-based and deterministic (P3): a task_type string is
//! grunt iff it is in the configured eligible list. No LLM decides routing —
//! the router itself must not cost tokens (§8.2 philosophy).

use std::sync::Arc;

use async_trait::async_trait;

use crate::inference::{InferenceProvider, InferenceRequest, InferenceResponse, ProviderError};

/// Conservative KV-cache footprint for the Q1 model
/// (Qwen2.5-Coder-7B llama.cpp defaults: 28 layers × 4 KV heads × 128 dim,
/// F16, both K and V ≈ 60 KB/token). Used only for the admission check.
pub const KV_BYTES_PER_TOKEN: u64 = 60 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkClass {
    /// Cheap, template-like, mechanical — local may serve it.
    Grunt,
    /// Reasoning-heavy (plan/code/verify/report) — cloud only.
    Core,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Local,
    Cloud { because: CloudReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudReason {
    /// task_type not in the eligible list.
    CoreTask,
    /// eligible but the model+context would not fit free VRAM (§10.1).
    VramRefused,
}

/// Deterministic classification: membership in the configured list, no LLM.
pub fn classify(task_type: &str, eligible: &[String]) -> WorkClass {
    if eligible.iter().any(|e| e == task_type) {
        WorkClass::Grunt
    } else {
        WorkClass::Core
    }
}

/// Rough token estimate: ~4 chars per token for the prompt side, plus the
/// requested output budget (§10.4's accounting is provider-reported; this is
/// only an admission heuristic).
pub fn estimate_ctx_tokens(req: &InferenceRequest) -> u64 {
    let prompt_chars = req.system_prefix.len() + req.context.len() + req.input.len() + 64; // framing
    (prompt_chars as u64) / 4 + req.max_tokens.unwrap_or(512) as u64
}

/// §10.1 VRAM admission: model load + KV cache for the requested context must
/// fit in free VRAM (all MiB). Equal/below fits; above does not.
pub fn vram_admits(free_mib: u64, model_load_mib: u64, ctx_tokens: u64) -> bool {
    let kv_mib = (ctx_tokens.saturating_mul(KV_BYTES_PER_TOKEN)) / (1024 * 1024);
    model_load_mib.saturating_add(kv_mib) <= free_mib
}

/// Pure routing decision given the current admission inputs.
pub fn decide(
    task_type: &str,
    eligible: &[String],
    free_mib: u64,
    model_load_mib: u64,
    ctx_tokens: u64,
) -> Route {
    match classify(task_type, eligible) {
        WorkClass::Core => Route::Cloud {
            because: CloudReason::CoreTask,
        },
        WorkClass::Grunt => {
            if vram_admits(free_mib, model_load_mib, ctx_tokens) {
                Route::Local
            } else {
                Route::Cloud {
                    because: CloudReason::VramRefused,
                }
            }
        }
    }
}

/// Wraps the two engines behind the single `InferenceProvider` face the env
/// layer and A10 selector sit on top of. Free-VRAM is read from `probe` at
/// call time so a full GPU silently degrades to cloud mid-run (§10.1).
pub struct LocalFirstRouting {
    local: Arc<dyn InferenceProvider>,
    cloud: Arc<dyn InferenceProvider>,
    eligible: Vec<String>,
    model_load_mib: u64,
    /// Free VRAM probe; `u64::MAX` keeps local always admitted (tests/offline).
    free_vram_mib: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl LocalFirstRouting {
    pub fn new(
        local: Arc<dyn InferenceProvider>,
        cloud: Arc<dyn InferenceProvider>,
        eligible: Vec<String>,
        model_load_mib: u64,
        free_vram_mib: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Self {
        Self {
            local,
            cloud,
            eligible,
            model_load_mib,
            free_vram_mib,
        }
    }

    /// Decision exposed for telemetry/resume logging (which arm SHOULD run).
    pub fn route_of(&self, req: &InferenceRequest) -> Route {
        decide(
            &req.task_type,
            &self.eligible,
            (self.free_vram_mib)(),
            self.model_load_mib,
            estimate_ctx_tokens(req),
        )
    }
}

#[async_trait]
impl InferenceProvider for LocalFirstRouting {
    fn name(&self) -> &'static str {
        "routing"
    }

    async fn complete(
        &self,
        request: &InferenceRequest,
    ) -> std::result::Result<InferenceResponse, ProviderError> {
        match self.route_of(request) {
            // Core / VRAM-refused: cloud never sees a local attempt.
            Route::Cloud { .. } => self.cloud.complete(request).await,
            Route::Local => match self.local.complete(request).await {
                Ok(resp) => Ok(resp),
                // Silent fallback (§10.1): local failure is an optimization
                // miss, not an incident — retry once on cloud, no escalation.
                // Silent to the run, visible in the tracing log (§15).
                Err(local_err) => {
                    tracing::warn!(
                        error = %local_err,
                        task_type = %request.task_type,
                        "local arm failed; falling back to cloud"
                    );
                    self.cloud.complete(request).await
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{MockProvider, MockStep};

    fn eligible() -> Vec<String> {
        [
            "boilerplate",
            "test_skeleton",
            "classification",
            "schema_repair",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    fn req(task_type: &str, max_tokens: Option<u32>) -> InferenceRequest {
        InferenceRequest {
            task_type: task_type.into(),
            system_prefix: "sys".into(),
            context: String::new(),
            input: "do the thing".into(),
            max_tokens,
        }
    }

    fn always_mock(text: &str, name: &'static str) -> Arc<dyn InferenceProvider> {
        struct Named(MockProvider, &'static str);
        #[async_trait]
        impl InferenceProvider for Named {
            fn name(&self) -> &'static str {
                self.1
            }
            async fn complete(
                &self,
                r: &InferenceRequest,
            ) -> std::result::Result<InferenceResponse, ProviderError> {
                let mut resp = self.0.complete(r).await?;
                // MockProvider stamps its own engine name; label the arm so
                // assertions can see which engine served the call.
                resp.provider = self.1.to_string();
                Ok(resp)
            }
        }
        Arc::new(Named(MockProvider::always(text), name))
    }

    // --- classification (rule-based, deterministic) ---

    #[test]
    fn classify_is_membership_and_total() {
        assert_eq!(classify("boilerplate", &eligible()), WorkClass::Grunt);
        assert_eq!(classify("classification", &eligible()), WorkClass::Grunt);
        assert_eq!(classify("plan", &eligible()), WorkClass::Core);
        assert_eq!(classify("code", &eligible()), WorkClass::Core);
        assert_eq!(classify("", &eligible()), WorkClass::Core);
        // totality/determinism fuzz: any task_type string, stable answer
        for s in [
            "a",
            "Z9",
            "test_skeleton ",
            " test_skeleton",
            "sınıflandırma",
            "🤖",
        ] {
            let a = classify(s, &eligible());
            let b = classify(s, &eligible());
            assert_eq!(a, b);
            assert_eq!(a, WorkClass::Core, "near-misses must not be grunt: {s:?}");
        }
    }

    // --- VRAM admission (§10.1) ---

    #[test]
    fn admission_boundary_is_exact_and_clamped() {
        // model 4800 MiB; KV = 10k × 60 KiB = 585 MiB (floor of integer div).
        assert!(vram_admits(8192, 4800, 10_000));
        assert!(vram_admits(5385, 4800, 10_000)); // 4800+585: exactly fits
        assert!(!vram_admits(5384, 4800, 10_000)); // one MiB short
        assert!(!vram_admits(1024, 4800, 0)); // model itself does not fit
        assert!(!vram_admits(0, 0, u64::MAX)); // saturating math, no panic
    }

    #[test]
    fn grunt_with_huge_context_is_refused_silently_to_cloud() {
        let r = decide(
            "boilerplate",
            &eligible(),
            8192,
            4800,
            estimate_ctx_tokens(&req("boilerplate", Some(256))),
        );
        assert_eq!(r, Route::Local);
        // 400k tokens ≈ 22 GiB KV → over budget: cloud, with VramRefused.
        let big = estimate_ctx_tokens(&req("boilerplate", Some(400_000)));
        let r = decide("boilerplate", &eligible(), 8192, 4800, big);
        assert_eq!(
            r,
            Route::Cloud {
                because: CloudReason::VramRefused
            }
        );
        let r = decide("plan", &eligible(), u64::MAX, 0, 0);
        assert_eq!(
            r,
            Route::Cloud {
                because: CloudReason::CoreTask
            }
        );
    }

    // --- routing behavior ---

    fn routing(
        local: Arc<dyn InferenceProvider>,
        cloud: Arc<dyn InferenceProvider>,
        free: u64,
    ) -> LocalFirstRouting {
        LocalFirstRouting::new(local, cloud, eligible(), 4800, Arc::new(move || free))
    }

    #[tokio::test]
    async fn grunt_goes_to_local_and_never_touches_cloud() {
        let cloud = always_mock("CLOUD", "cloud");
        let local = always_mock("LOCAL", "local");
        let out = routing(local, cloud.clone(), 8192)
            .complete(&req("boilerplate", Some(256)))
            .await
            .unwrap();
        assert_eq!(out.text, "LOCAL");
        assert_eq!(out.provider, "local");
        assert_eq!(cloud.name(), "cloud"); // cloud existed but was not consulted
    }

    #[tokio::test]
    async fn core_task_never_attempts_local() {
        let cloud = always_mock("CLOUD", "cloud");
        let local = always_mock("LOCAL", "local");
        let out = routing(local, cloud, 8192)
            .complete(&req("code", Some(256)))
            .await
            .unwrap();
        assert_eq!(out.text, "CLOUD");
        assert_eq!(out.provider, "cloud");
    }

    #[tokio::test]
    async fn local_failure_falls_back_to_cloud_silently() {
        // local script: first call fails with timeout, then would loop success.
        let local: Arc<dyn InferenceProvider> = Arc::new(MockProvider::new(vec![
            MockStep::Fail(ProviderError::Timeout),
            MockStep::Success {
                text: "L2".into(),
                tokens_in: 1,
                tokens_out: 1,
            },
        ]));
        let cloud = always_mock("CLOUD", "cloud");
        let out = routing(local, cloud, 8192)
            .complete(&req("classification", Some(256)))
            .await
            .unwrap();
        assert_eq!(out.text, "CLOUD"); // no escalation, no error surfaced
        assert_eq!(out.provider, "cloud");
    }

    #[tokio::test]
    async fn vram_refused_grunt_skips_local_attempt_entirely() {
        // 8 GiB free, 4.8 GiB model, request wants ~400k-token context.
        let cloud = always_mock("CLOUD", "cloud");
        let local = always_mock("LOCAL", "local");
        let r = routing(local, cloud, 8192);
        let out = r
            .complete(&req("schema_repair", Some(400_000)))
            .await
            .unwrap();
        assert_eq!(out.text, "CLOUD");
        assert_eq!(r.route_of(&req("schema_repair", Some(256))), Route::Local);
    }

    #[tokio::test]
    async fn free_vram_change_between_calls_changes_route() {
        // probe is read per call: a filling GPU mid-run degrades later calls.
        use std::sync::atomic::{AtomicU64, Ordering};
        let free = Arc::new(AtomicU64::new(8192));
        let local = always_mock("LOCAL", "local");
        let cloud = always_mock("CLOUD", "cloud");
        let probe_free = free.clone();
        let r = LocalFirstRouting::new(
            local,
            cloud,
            eligible(),
            4800,
            Arc::new(move || probe_free.load(Ordering::Relaxed)),
        );
        assert_eq!(r.route_of(&req("boilerplate", Some(256))), Route::Local);
        free.store(1024, Ordering::Relaxed); // other process ate the VRAM
        assert_eq!(
            r.route_of(&req("boilerplate", Some(256))),
            Route::Cloud {
                because: CloudReason::VramRefused
            }
        );
    }
}
