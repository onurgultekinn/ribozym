//! Cloud provider — OpenRouter, OpenAI-compatible (§10.1, primary engine).
//! Rate limits via the shared env layer; secrets come only from env
//! (`OPENROUTER_API_KEY` — P7: no keys in code).

use async_trait::async_trait;

use super::{InferenceProvider, InferenceRequest, InferenceResponse, Usage};
use crate::inference::ProviderError;

pub struct CloudProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

/// Shared [OI]-compatible response shape; local.rs (same protocol) reuses this.
#[derive(serde::Deserialize)]
pub struct ApiResponseShape {
    choices: Vec<ApiChoice>,
    #[serde(default)]
    usage: Option<ApiUsage>,
    #[serde(default)]
    model: Option<String>,
}

impl ApiResponseShape {
    pub fn text(&self) -> String {
        self.choices
            .first()
            .and_then(|c| c.message.as_ref())
            .and_then(|m| m.content.clone())
            .unwrap_or_default()
    }

    pub fn usage(&self) -> Option<Usage> {
        self.usage.clone().map(|u| Usage {
            tokens_in: u.prompt_tokens,
            tokens_out: u.completion_tokens,
            cached_tokens: u
                .prompt_tokens_details
                .map(|d| d.cached_tokens)
                .unwrap_or(0),
        })
    }

    pub fn served_model(&self) -> Option<String> {
        self.model.clone()
    }

    /// Assemble the provider-agnostic response (shared by cloud and local —
    /// same wire protocol, same assembly, §10.1).
    pub fn to_response(&self, provider: &str, fallback_model: &str) -> InferenceResponse {
        InferenceResponse {
            text: self.text(),
            usage: self.usage().unwrap_or(Usage {
                tokens_in: 0,
                tokens_out: 0,
                cached_tokens: 0,
            }),
            provider: provider.to_string(),
            model: self
                .served_model()
                .unwrap_or_else(|| fallback_model.to_string()),
        }
    }
}

#[derive(serde::Deserialize)]
struct ApiChoice {
    #[serde(default)]
    message: Option<ApiMessage>,
}

#[derive(serde::Deserialize)]
struct ApiMessage {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Clone, serde::Deserialize)]
struct ApiUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    prompt_tokens_details: Option<ApiCachedDetail>,
}

#[derive(Clone, serde::Deserialize)]
struct ApiCachedDetail {
    #[serde(default)]
    cached_tokens: u64,
}

impl CloudProvider {
    /// Build from registry config. `api_key` is resolved from the environment
    /// by the caller; an empty key constructs fine but the first call fails
    /// (config validate already flags missing keys before any run starts).
    pub fn new(base_url: String, api_key: String, model: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
            api_key,
            model,
        }
    }
}

#[async_trait]
impl InferenceProvider for CloudProvider {
    fn name(&self) -> &'static str {
        "openrouter"
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

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .header("HTTP-Referer", "https://github.com/ribozym")
            .header("X-Title", "ribozym")
            .json(&body)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ProviderError::Timeout
                } else {
                    ProviderError::Unavailable(e.to_string())
                }
            })?;

        let status = resp.status().as_u16();
        if status == 429 {
            let retry_after_ms = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(|s| s * 1000)
                .unwrap_or(5000);
            return Err(ProviderError::RateLimited { retry_after_ms });
        }
        if status >= 400 {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Http {
                status,
                body: body_text,
            });
        }

        let parsed: ApiResponseShape = resp
            .json()
            .await
            .map_err(|e| ProviderError::Unavailable(format!("bad response: {e}")))?;

        Ok(parsed.to_response(self.name(), &self.model))
    }
}
