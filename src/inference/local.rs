//! Local provider — llama-server (llama.cpp), [OI]-compatible
//! `/v1/chat/completions` (§10.1, auxiliary offload — never required; Q1).
//! Same wire protocol as cloud; different endpoint, no auth, no cost.

use async_trait::async_trait;

use super::{InferenceProvider, InferenceRequest, InferenceResponse};
use crate::inference::ProviderError;

pub struct LocalProvider {
    http: reqwest::Client,
    server_url: String,
    model: String,
}

impl LocalProvider {
    pub fn new(server_url: String, model: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            server_url,
            model,
        }
    }
}

#[async_trait]
impl InferenceProvider for LocalProvider {
    fn name(&self) -> &'static str {
        "local"
    }

    async fn complete(
        &self,
        request: &InferenceRequest,
    ) -> std::result::Result<InferenceResponse, ProviderError> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": request.system_prefix},
                {"role": "user", "content": format!("{}\n\n{}", request.context, request.input)}
            ],
            "max_tokens": request.max_tokens
        });

        let url = format!(
            "{}/v1/chat/completions",
            self.server_url.trim_end_matches('/')
        );
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .timeout(std::time::Duration::from_secs(300))
            .send()
            .await
            .map_err(|e| {
                // Local is an optimization, never a requirement (§10.2):
                // unhealthy local is surfaced as Unavailable; the router
                // falls back to cloud without counting it as E0.
                if e.is_timeout() {
                    ProviderError::Timeout
                } else {
                    ProviderError::Unavailable(e.to_string())
                }
            })?;

        let status = resp.status().as_u16();
        if status >= 400 {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Http {
                status,
                body: body_text,
            });
        }

        let parsed: super::cloud::ApiResponseShape = resp
            .json()
            .await
            .map_err(|e| ProviderError::Unavailable(format!("bad response: {e}")))?;

        Ok(parsed.to_response(self.name(), &self.model))
    }
}
