//! A4 — Test Architect (§5.9, KİLİTLİ). Owner of `tests/` — the ONLY agent
//! allowed to write there (A5 is refused at the path layer).
//!
//! Pipeline (§5.9 m1/m4, v2.34 yazım/koşum ayrımı):
//!   LLM turn (schema-forced `coding_task.schema.json`, test files constrained
//!   to `tests/`) → write into the isolated workdir's tests/ → RUN them via
//!   `cargo test` through the ToolGateway (no LLM needed to RUN, §5.9 m4) →
//!   ExecutionResult.
//!
//! Hakkaniyet (§5.9 m5): the model is prompted with the task's acceptance
//! criteria as the test's PURPOSE, and the test files must cite the criterion
//! id in a doc-comment (`// AC1:`) — a test that does not reference any
//! criterion id is rejected before writing (untraceable test = the
//! "written-to-pass" shield the plan forbids).

use async_trait::async_trait;

use crate::agent::{Agent, AgentContext, TaskInput};
use crate::error::{Error, Result};
use crate::models::execution::{CommandRun, ExecutionResult, LogEntry, ModelUsage};
use crate::models::task::TaskItem;

#[derive(Default)]
pub struct A4TestArchitect;

impl A4TestArchitect {
    pub fn new() -> Self {
        Self
    }

    fn system_prefix() -> String {
        crate::config::agents_prompt_from(std::path::Path::new("."), "A4_test_architect")
            .unwrap_or_else(|_| "You are A4, the Test Architect.".into())
    }

    /// tests/-relative path check: MIRROR of A5's, but inverted ownership —
    /// here ONLY tests/ is allowed (§5.9 m1).
    fn safe_test_path(workdir: &std::path::Path, rel: &str) -> Result<std::path::PathBuf> {
        if rel.contains('\\') || rel.starts_with('/') || rel.contains("..") {
            return Err(Error::Agent(format!("unsafe test path {rel:?}")));
        }
        if !rel.starts_with("tests/") {
            return Err(Error::Agent(format!(
                "A4 writes ONLY under tests/; got {rel:?} (§5.9 m1)"
            )));
        }
        let p = workdir.join(rel);
        let norm: std::path::PathBuf = p
            .components()
            .filter(|c| !matches!(c, std::path::Component::CurDir))
            .collect();
        if !norm.starts_with(workdir) {
            return Err(Error::Agent(format!("test escapes workdir: {rel:?}")));
        }
        Ok(norm)
    }

    /// Every criterion id the task carries.
    fn criterion_ids(task: &TaskItem) -> Vec<String> {
        // Acceptance is a single block in the model (§11.2); the AC id lives
        // in extra when the plan supplies one (`acceptance_ids: ["AC1"]`).
        task.extra
            .get("acceptance_ids")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_else(|| vec![task.id.clone()])
    }

    fn parse_output(task: &TaskItem, value: serde_json::Value) -> Result<Vec<(String, String)>> {
        let got = value.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
        if got != task.id {
            return Err(Error::Agent(format!(
                "test output task_id {got:?} != dispatched {:?}",
                task.id
            )));
        }
        let files: Vec<(String, String)> = value
            .get("files")
            .and_then(|f| f.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|f| {
                        Some((
                            f.get("path")?.as_str()?.to_string(),
                            f.get("content")?.as_str()?.to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if files.is_empty() {
            return Err(Error::Agent("test output carried no files".into()));
        }
        // §5.9 m5 (a): every produced test file must CITE at least one
        // criterion id — else it is an untraceable, possibly unfair test.
        let ids = Self::criterion_ids(task);
        for (path, content) in &files {
            let cited = ids.iter().any(|id| content.contains(id));
            if !cited {
                return Err(Error::Agent(format!(
                    "test file {path:?} cites none of {ids:?} — untraceable test rejected (§5.9 m5)"
                )));
            }
        }
        Ok(files)
    }
}

#[async_trait]
impl Agent for A4TestArchitect {
    fn id(&self) -> &'static str {
        "A4"
    }
    fn title(&self) -> &'static str {
        "Test Architect"
    }

    async fn run_task(&self, ctx: &AgentContext, input: &TaskInput) -> Result<ExecutionResult> {
        let started = crate::clock::now_utc_iso8601();
        let task = &input.task;
        let ids = Self::criterion_ids(task);
        let req = crate::inference::InferenceRequest {
            task_type: "test_skeleton".into(),
            system_prefix: Self::system_prefix(),
            context: input.context.clone(),
            input: format!(
                "TASK {}\nTITLE: {}\nACCEPTANCE CRITERIA (tests MUST cite these ids in doc-comments): {}\nCRITERIA: {}\nWrite integration tests under tests/ that genuinely verify the criteria against src/lib.rs.",
                task.id, task.title, ids.join(", "), task.acceptance.criteria
            ),
            max_tokens: Some(4096),
        };
        let out = self
            .complete_json(ctx, "coding_task.schema.json", &req)
            .await?;
        let turns = out.turns;
        let response = out.response.clone();
        let files = Self::parse_output(task, out.value)?;

        let mut artifacts = Vec::new();
        for (rel, content) in &files {
            let full = Self::safe_test_path(&ctx.workdir, rel)?;
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&full, content)?;
            artifacts.push(crate::models::execution::Artifact {
                kind: "test_file".into(),
                path: rel.clone(),
                hash: format!("sha256:{}", crate::hash::sha256_hex(content.as_bytes())),
                size_bytes: content.len() as u64,
            });
        }

        // §5.9 m4: RUNNING tests needs no LLM — real `cargo test` via gateway.
        let (status, summary, commands): (String, String, Vec<CommandRun>) =
            match ctx.gateway.run("cargo test") {
                Ok(out) => {
                    let cmd = CommandRun {
                        cmd: "cargo test".into(),
                        exit_code: out.exit_code,
                        duration_ms: out.duration_ms,
                    };
                    if out.exit_code == 0 {
                        (
                            "success".into(),
                            format!("tests written: {} file(s); cargo test PASSED", files.len()),
                            vec![cmd],
                        )
                    } else {
                        let tail = if out.stderr.len() > 1500 {
                            &out.stderr[out.stderr.len() - 1500..]
                        } else {
                            &out.stderr
                        };
                        (
                            "failed".into(),
                            format!("tests written but cargo test FAILED:\n{tail}"),
                            vec![cmd],
                        )
                    }
                }
                Err(e) => {
                    // §5.9 m4 stands: if the toolchain is missing the task CANNOT
                    // claim verified tests — fail honestly.
                    (
                        "failed".into(),
                        format!("cargo test refused by gateway: {e}"),
                        vec![],
                    )
                }
            };

        let usage = &response.usage;
        let model_usage = Some(ModelUsage {
            provider: response.provider.clone(),
            model: response.model.clone(),
            tokens_in: usage.tokens_in,
            tokens_out: usage.tokens_out,
            cached_tokens: usage.cached_tokens,
            cost_usd: crate::agents::a5::estimate_cost(
                &response.provider,
                usage.tokens_in,
                usage.tokens_out,
            ),
            cache_savings_usd: 0.0,
        });

        Ok(ExecutionResult {
            schema_version: "1.0".into(),
            task_id: task.id.clone(),
            agent_id: ctx.agent_id.clone(),
            started_at: started,
            finished_at: crate::clock::now_utc_iso8601(),
            status: status.clone(),
            summary,
            artifacts,
            logs: vec![LogEntry {
                level: "info".into(),
                message: format!("tests cite criteria {}", ids.join(", ")),
                ts: crate::clock::now_utc_iso8601(),
            }],
            model_usage,
            toolchain_used: Some("cargo".into()),
            commands_run: commands,
            next_agent: if status == "success" {
                Some("A3".into())
            } else {
                None
            },
            error: None,
            escalate_to_ceo: false,
            human_approval_needed: status != "success",
            idempotency: None,
            extra: {
                let mut m = serde_json::Map::new();
                m.insert("llm_turns".into(), serde_json::json!(turns));
                m
            },
        })
    }
}
