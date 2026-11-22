//! A11 — Web Porter & Blind Verifier (§13, v2.45 two roles one agent).
//! The system's ONLY door for external CONTENT (model calls and price
//! lookups are control-plane plumbing, exempt per Q2). MVP per the Q2
//! decision: the verification is the RULE layer only — `rules_only: true`,
//! LLM blind scoring opens at v1.0 (§13.3).
//!
//! Deterministic agent: no LLM turn, so `model_usage: null` (§11.3). Its
//! task description carries (or IS) the WebQuery; the query is validated
//! against `web_query.schema.json` BEFORE any fetch, the results against
//! `web_finding` / `verification_score` before they are written. Fetched
//! text NEVER flows onward — sources are recorded as URL + domain + type +
//! snippet HASH (P7: web content is data, and this keeps it that way).
//!
//! The door itself (backend, cache, scoring) is `crate::web::WebDoor`,
//! injected via the bridge so tests script fetches (§21.1) and production
//! routes them through the configured backend.

use async_trait::async_trait;
use serde_json::json;

use crate::agent::{Agent, AgentContext, TaskInput};
use crate::error::{Error, Result};
use crate::hash;
use crate::models::execution::{Artifact, ExecutionResult, LogEntry};
use crate::validate;

pub struct A11Web;

impl A11Web {
    pub fn new() -> Self {
        Self
    }

    /// The WebQuery: JSON inside the description (prose may surround it —
    /// `extract_json` is the lenient reader, the schema is the strict gate);
    /// absent JSON means the whole description IS the question.
    fn query_of(task_id: &str, description: &str) -> Result<serde_json::Value> {
        let q = match validate::extract_json(description) {
            Some(v) => v,
            None => {
                let ask = description.trim();
                if ask.is_empty() {
                    return Err(Error::Agent(
                        "A11 task carries neither a WebQuery JSON nor a question".into(),
                    ));
                }
                json!({ "id": format!("q-{}", task_id.trim_start_matches('T')), "ask": ask })
            }
        };
        Ok(q)
    }
}

impl Default for A11Web {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Agent for A11Web {
    fn id(&self) -> &'static str {
        "A11"
    }
    fn title(&self) -> &'static str {
        "Web Porter & Blind Verifier"
    }

    async fn run_task(&self, ctx: &AgentContext, input: &TaskInput) -> Result<ExecutionResult> {
        let started = crate::clock::now_utc_iso8601();
        let door = ctx.web.clone().ok_or_else(|| {
            Error::WebUnavailable("no web door injected (bridge .with_web)".into())
        })?;
        let query = A11Web::query_of(&input.task.id, &input.task.description)?;
        let qid = query["id"].as_str().unwrap_or("q-1").to_string();
        let (finding, score) = door.search(&query)?;

        // Land both records in the workdir (workdir-relative by construction:
        // qid passed the `^q-[0-9]+$` pattern inside the schema gate).
        let mut artifacts = Vec::new();
        for (label, doc) in [("findings", &finding), ("score", &score)] {
            let rel = format!("web/{label}-{qid}.json");
            let full = crate::agents::a5::A5Coder::safe_path_pub(&ctx.workdir, &rel)?;
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let bytes = hash::canonical_json(doc).into_bytes();
            std::fs::write(&full, &bytes)?;
            artifacts.push(Artifact {
                kind: "web_record".into(),
                path: rel,
                hash: format!("sha256:{}", hash::sha256_hex(&bytes)),
                size_bytes: bytes.len() as u64,
            });
        }
        let n_sources = finding["sources"].as_array().map(|a| a.len()).unwrap_or(0);
        let mut extra = serde_json::Map::new();
        extra.insert("web_query_id".into(), json!(qid));
        extra.insert("confidence".into(), score["confidence"].clone());
        extra.insert("behavior".into(), score["behavior"].clone());
        extra.insert("rules_only".into(), json!(true));
        extra.insert("llm_turns".into(), json!(0));
        Ok(ExecutionResult {
            schema_version: "1.0".into(),
            task_id: input.task.id.clone(),
            agent_id: ctx.agent_id.clone(),
            started_at: started,
            finished_at: crate::clock::now_utc_iso8601(),
            status: "success".into(),
            summary: format!(
                "web {qid}: {n_sources} sources, confidence {} (rules-only), finding {}",
                score["confidence"].as_str().unwrap_or("?"),
                artifacts[0].path
            ),
            artifacts,
            logs: vec![LogEntry {
                level: "info".into(),
                message: "fetched content stored as hashes only (P7)".into(),
                ts: crate::clock::now_utc_iso8601(),
            }],
            // Deterministic (§11.3): A11's MVP half makes no model call.
            model_usage: None,
            toolchain_used: Some("web".into()),
            commands_run: vec![],
            next_agent: None,
            error: None,
            escalate_to_ceo: false,
            human_approval_needed: false,
            idempotency: None,
            extra,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_extraction_prefers_json_then_falls_back_to_prose() {
        let q = A11Web::query_of(
            "T5",
            "please research: {\"id\": \"q-7\", \"ask\": \"axum streams\", \"min_sources\": 2}",
        )
        .unwrap();
        assert_eq!(q["id"], "q-7");
        let q = A11Web::query_of("T5", "which http client is fastest?").unwrap();
        assert_eq!(q["id"], "q-5", "id derived from the task, honestly");
        assert_eq!(q["ask"], "which http client is fastest?");
        assert!(A11Web::query_of("T5", "   ").is_err());
    }
}
