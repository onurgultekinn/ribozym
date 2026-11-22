//! MockLLM — test-time provider (§21.1). Never used in production code paths.
//!
//! Scenarios come from a scripted list; each entry is either a successful
//! response or a provider error to emit once. This mirrors the wiremock
//! JSONL approach: every schema-invalid response, timeout and 429 scenario
//! is expressible as data, not code.

use async_trait::async_trait;

use super::{InferenceProvider, InferenceRequest, InferenceResponse, ProviderError, Usage};
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub enum MockStep {
    /// Text + usage; served model defaults to "mock/mock-1".
    Success {
        text: String,
        tokens_in: u64,
        tokens_out: u64,
    },
    Fail(ProviderError),
}

pub struct MockProvider {
    steps: Mutex<Vec<MockStep>>,
    /// Calls seen so far — assertions inspect this (prompt shape, task_type...).
    pub calls: Mutex<Vec<InferenceRequest>>,
}

impl MockProvider {
    pub fn new(steps: Vec<MockStep>) -> Self {
        Self {
            steps: Mutex::new(steps),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Convenience: always-success provider returning fixed text.
    pub fn always(text: &str) -> Self {
        Self::new(vec![MockStep::Success {
            text: text.to_string(),
            tokens_in: 100,
            tokens_out: 50,
        }])
    }

    fn next_step(&self) -> std::result::Result<MockStep, ProviderError> {
        let mut queue = self.steps.lock().unwrap();
        if queue.is_empty() {
            // Exhausted script: replay last success if any, else fail closed.
            return Err(ProviderError::Unavailable("mock script exhausted".into()));
        }
        // Steps are consumed; a single-step script loops (always-success case).
        if queue.len() == 1 {
            return Ok(queue[0].clone());
        }
        Ok(queue.remove(0))
    }
}

#[async_trait]
impl InferenceProvider for MockProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn complete(
        &self,
        request: &InferenceRequest,
    ) -> std::result::Result<InferenceResponse, ProviderError> {
        self.calls.lock().unwrap().push(request.clone());
        match self.next_step()? {
            MockStep::Success {
                text,
                tokens_in,
                tokens_out,
            } => Ok(InferenceResponse {
                text,
                usage: Usage {
                    tokens_in,
                    tokens_out,
                    cached_tokens: 0,
                },
                provider: self.name().to_string(),
                model: "mock/mock-1".to_string(),
            }),
            MockStep::Fail(e) => Err(e),
        }
    }
}
