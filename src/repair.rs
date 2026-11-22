//! Schema-guarded completion — the one-turn repair loop (§14, §16
//! MODEL_SCHEMA_INVALID). The control plane never trusts provider text: the
//! response must yield a JSON object that passes FULL schema validation.
//! A rejected output gets exactly one repair turn (JSON extraction from
//! prose + the schema-diff message appended to the dynamic part of the
//! prompt; the fixed prefix stays byte-stable for §10.4 caching). A second
//! failure surfaces MODEL_SCHEMA_INVALID — E0 counting joins in Phase 4.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::inference::{InferenceProvider, InferenceRequest, InferenceResponse};
use crate::validate;

pub struct SchemaGuard {
    provider: Arc<dyn InferenceProvider>,
    /// Directory holding the schemas/ contracts (usually <repo>/schemas).
    schema_dir: PathBuf,
}

#[derive(Debug)]
pub struct GuardedOutput {
    pub response: InferenceResponse,
    /// The schema-valid JSON object parsed out of `response.text`.
    pub value: Value,
    /// 1 = accepted on the first try, 2 = accepted after the repair turn.
    pub turns: u32,
    /// Violations that triggered the repair turn; empty on first-try pass.
    pub first_violations: Vec<String>,
}

impl SchemaGuard {
    pub fn new(provider: Arc<dyn InferenceProvider>, schema_dir: impl Into<PathBuf>) -> Self {
        Self {
            provider,
            schema_dir: schema_dir.into(),
        }
    }

    /// One guarded completion with at most one repair turn.
    pub async fn complete(
        &self,
        request: &InferenceRequest,
        schema_name: &str,
    ) -> Result<GuardedOutput> {
        let first = self.provider.complete(request).await?;
        match self.check(schema_name, &first.text) {
            Ok(value) => Ok(GuardedOutput {
                response: first,
                value,
                turns: 1,
                first_violations: Vec::new(),
            }),
            Err(violations) => {
                let repair = self.repair_request(request, &first.text, &violations);
                let second = self.provider.complete(&repair).await?;
                match self.check(schema_name, &second.text) {
                    Ok(value) => Ok(GuardedOutput {
                        response: second,
                        value,
                        turns: 2,
                        first_violations: violations,
                    }),
                    Err(second_violations) => Err(Error::ModelSchemaInvalid(format!(
                        "schema '{schema_name}' still invalid after repair turn: {}; first pass: {}",
                        second_violations.join("; "),
                        violations.join("; ")
                    ))),
                }
            }
        }
    }

    fn check(&self, schema_name: &str, text: &str) -> std::result::Result<Value, Vec<String>> {
        let value = validate::extract_json(text)
            .ok_or_else(|| vec![format!("no JSON object found in response: {text:.200}")])?;
        let problems = validate::validate_full(&self.schema_dir, schema_name, &value);
        if problems.is_empty() {
            Ok(value)
        } else {
            Err(problems)
        }
    }

    /// Repair turn prompt: rejected text + schema diff appended to the
    /// DYNAMIC input; system_prefix and context stay byte-identical (§10.4).
    fn repair_request(
        &self,
        original: &InferenceRequest,
        bad_text: &str,
        violations: &[String],
    ) -> InferenceRequest {
        let mut req = original.clone();
        req.input = format!(
            "{}\n\n--- PREVIOUS RESPONSE (REJECTED) ---\n{}\n\n--- SCHEMA VIOLATIONS ---\n- {}\n\nReturn ONLY the corrected JSON object. No prose, no code fences.",
            original.input,
            bad_text,
            violations.join("\n- ")
        );
        req
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{MockProvider, MockStep};

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn valid_event() -> String {
        r#"{"seq":1,"ts":"2026-09-12T10:00:00Z","spec_id":"spec-20260912-001","run_id":"run-x1","event":"TaskStarted","correlation_id":"op-1","schema_version":"1.0"}"#
            .to_string()
    }

    /// Schema-invalid: missing seq, correlation_id, schema_version + bad id pattern.
    fn invalid_event_wrapped() -> String {
        "Here is my plan:\n{\"ts\":\"2026-09-12T10:00:00Z\",\"spec_id\":\"spec-20260912-001\",\"run_id\":\"run-x1\",\"event\":\"TaskStarted\"}\nDone."
            .to_string()
    }

    fn step(text: &str) -> MockStep {
        MockStep::Success {
            text: text.to_string(),
            tokens_in: 100,
            tokens_out: 50,
        }
    }

    fn request() -> InferenceRequest {
        InferenceRequest {
            task_type: "test".into(),
            system_prefix: "SYSTEM PROMPT (fixed prefix)".into(),
            context: "session context".into(),
            input: "produce an event envelope".into(),
            max_tokens: Some(256),
        }
    }

    #[tokio::test]
    async fn first_try_valid_single_call() {
        let mock = MockProvider::new(vec![step(&valid_event())]);
        let guard = SchemaGuard::new(Arc::new(mock), repo_root());
        let out = guard
            .complete(&request(), "event.schema.json")
            .await
            .unwrap();
        assert_eq!(out.turns, 1);
        assert!(out.first_violations.is_empty());
        assert_eq!(out.value["event"], "TaskStarted");
    }

    #[tokio::test]
    async fn repair_succeeds_on_second_turn() {
        let mock = MockProvider::new(vec![step(&invalid_event_wrapped()), step(&valid_event())]);
        let guard = SchemaGuard::new(Arc::new(mock), repo_root());
        let out = guard
            .complete(&request(), "event.schema.json")
            .await
            .unwrap();
        assert_eq!(out.turns, 2);
        assert_eq!(out.value["seq"], 1);
        assert!(
            !out.first_violations.is_empty(),
            "the violations that triggered repair must be recorded"
        );
    }

    #[tokio::test]
    async fn second_failure_surfaces_model_schema_invalid() {
        let mock = MockProvider::new(vec![
            step(&invalid_event_wrapped()),
            step("still broken: {\"ts\":\"x\"}"),
        ]);
        let guard = SchemaGuard::new(Arc::new(mock), repo_root());
        let err = guard
            .complete(&request(), "event.schema.json")
            .await
            .unwrap_err();
        assert!(matches!(err, Error::ModelSchemaInvalid(_)), "got: {err}");
    }

    #[tokio::test]
    async fn repair_turn_keeps_prefix_stable_and_cites_violations() {
        let mock = MockProvider::new(vec![step(&invalid_event_wrapped()), step(&valid_event())]);
        // Keep a direct handle to inspect calls; the Arc is a second clone of
        // the same provider.
        let mock = Arc::new(mock);
        let calls_ref = Arc::clone(&mock);
        let guard = SchemaGuard::new(mock, repo_root());
        let _ = guard
            .complete(&request(), "event.schema.json")
            .await
            .unwrap();
        let calls = calls_ref.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0].system_prefix, calls[1].system_prefix,
            "fixed prefix must stay byte-stable across the repair turn (§10.4)"
        );
        assert_eq!(calls[0].context, calls[1].context);
        assert!(
            calls[1].input.contains("SCHEMA VIOLATIONS"),
            "repair input must carry the schema diff"
        );
        assert!(
            calls[1].input.contains("PREVIOUS RESPONSE (REJECTED)"),
            "repair input must carry the rejected text"
        );
        assert!(
            calls[1].input.starts_with("produce an event envelope"),
            "original user input must be preserved, not replaced"
        );
    }
}
