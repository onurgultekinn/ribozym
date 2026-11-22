//! A5 — Lead Coder (§5.10, KİLİTLİ). One job: turn an assigned task into
//! working code INSIDE the locked ADR frame.
//!
//! The pipeline per task (§6.3 template, code lane):
//!   LLM turn (schema-forced `coding_task.schema.json`) → write files into the
//!   ISOLATED workdir (path-checked: no traversal, workdir-relative) → run the
//!   task's `self_check.commands` through the ToolGateway (allowlist; §12) →
//!   on failure, ONE fix loop iteration (≤3 total, §8.1 E1) with the tool
//!   output fed back → produce `ExecutionResult`.
//!
//! Hard rules enforced HERE, not trusted from the model:
//! * `tests/` is FORBIDDEN (A4's property, §5.9) — a model that writes there
//!   gets the file dropped and the turn fails, not a warning.
//! * Only workdir-relative paths; `..`/absolute are rejected by the schema AND
//!   re-checked before any byte is written (defense in depth, §12.6).
//! * The LLM never runs commands itself: `self_check.commands` are only hints
//!   intersected with the toolchain allowlist via the gateway.

use std::path::Path;

use async_trait::async_trait;
use serde_json::json;

use crate::agent::{Agent, AgentContext, TaskInput};
use crate::error::{Error, Result};
use crate::gateway::ToolOutput;
use crate::hash;
use crate::inference::InferenceRequest;
use crate::models::execution::{Artifact, CommandRun, ExecutionResult, LogEntry, ModelUsage};
use crate::models::task::TaskItem;

/// §8.1 E1: A4/A7 caught errors go back to A5 at most 3 iterations.
pub const FIX_LOOP_MAX: u32 = 3;

#[derive(Default)]
pub struct A5Coder;

/// One validated file patch from the model output.
struct Patch {
    path: String,
    content: String,
}

/// Model output after schema validation (coding_task.schema.json).
struct CodingOutput {
    patches: Vec<Patch>,
    self_check: Vec<String>,
    notes: String,
}

impl A5Coder {
    pub fn new() -> Self {
        Self
    }

    /// The byte-stable system prefix for A5 turns (§10.4): role frame + hard
    /// rules. Built once per call from the locked prompt text — never embeds
    /// task-specific data (that goes in `context`/`input`).
    fn system_prefix() -> String {
        crate::config::agents_prompt_from(Path::new("."), "A5_coder")
            .unwrap_or_else(|_| "You are A5, the Lead Coder.".into())
    }

    /// Schema-validate the raw model text into structured patches.
    fn parse_output(task_id: &str, value: serde_json::Value) -> Result<CodingOutput> {
        // task_id must match the dispatched task — a model answering a
        // different task id is answering a different question.
        let got = value.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
        if got != task_id {
            return Err(Error::Agent(format!(
                "coding output task_id {got:?} != dispatched {task_id:?}"
            )));
        }
        let patches = value
            .get("files")
            .and_then(|f| f.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|f| {
                        Some(Patch {
                            path: f.get("path")?.as_str()?.to_string(),
                            content: f.get("content")?.as_str()?.to_string(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if patches.is_empty() {
            return Err(Error::Agent("coding output carried no files".into()));
        }
        let self_check = value
            .get("self_check")
            .and_then(|s| s.get("commands"))
            .and_then(|c| c.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|c| c.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(CodingOutput {
            patches,
            self_check,
            notes: value
                .get("notes")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string(),
        })
    }

    /// Public re-export of the workdir path discipline for other file-writing
    /// agents (A11's finding records); identical rules, one implementation.
    pub fn safe_path_pub(workdir: &Path, rel: &str) -> Result<std::path::PathBuf> {
        Self::safe_path(workdir, rel)
    }

    /// §5.9 + §12.6 enforcement before any byte hits disk: workdir-relative,
    /// no `..`, no absolute, and never inside `tests/`.
    fn safe_path(workdir: &Path, rel: &str) -> Result<std::path::PathBuf> {
        if rel.contains('\\') || rel.starts_with('/') || rel.contains("..") {
            return Err(Error::Agent(format!("unsafe artifact path {rel:?}")));
        }
        let p = workdir.join(rel);
        let norm = p
            .components()
            .filter(|c| !matches!(c, std::path::Component::CurDir))
            .collect::<std::path::PathBuf>();
        if !norm.starts_with(workdir) {
            return Err(Error::Agent(format!("artifact escapes workdir: {rel:?}")));
        }
        if let Some(first) = rel.split('/').next() {
            if first == "tests" {
                return Err(Error::Agent(format!(
                    "tests/ is A4's property; A5 wrote {rel:?} (§5.9)"
                )));
            }
        }
        Ok(norm)
    }

    fn write_patches(workdir: &Path, patches: &[Patch]) -> Result<Vec<Artifact>> {
        let mut artifacts = Vec::new();
        for p in patches {
            let full = Self::safe_path(workdir, &p.path)?;
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&full, &p.content)?;
            artifacts.push(Artifact {
                kind: "source_file".into(),
                path: p.path.clone(),
                hash: format!("sha256:{}", hash::sha256_hex(p.content.as_bytes())),
                size_bytes: p.content.len() as u64,
            });
        }
        Ok(artifacts)
    }

    /// Run the model's suggested self-check commands through the gateway.
    /// Returns (all_ok, command records, first failure output snippet).
    fn self_check(ctx: &AgentContext, commands: &[String]) -> (bool, Vec<CommandRun>, String) {
        let mut runs = Vec::new();
        for cmd in commands {
            // The gateway is the ONLY door: allowlist + isolated env + timeout.
            match ctx.gateway.run(cmd) {
                Ok(ToolOutput {
                    exit_code,
                    stderr,
                    duration_ms,
                    ..
                }) => {
                    let ok = exit_code == 0;
                    runs.push(CommandRun {
                        cmd: cmd.clone(),
                        exit_code,
                        duration_ms,
                    });
                    if !ok {
                        let snippet = if stderr.len() > 2000 {
                            &stderr[..2000]
                        } else {
                            &stderr
                        };
                        return (false, runs, format!("{cmd} exited {exit_code}: {snippet}"));
                    }
                }
                Err(e) => {
                    runs.push(CommandRun {
                        cmd: cmd.clone(),
                        exit_code: -1,
                        duration_ms: 0,
                    });
                    return (false, runs, format!("{cmd} refused: {e}"));
                }
            }
        }
        (true, runs, String::new())
    }

    fn request(task: &TaskItem, input: &TaskInput, feedback: Option<&str>) -> InferenceRequest {
        let mut body = format!(
            "TASK {}\nTITLE: {}\nDESCRIPTION: {}\nACCEPTANCE: {}\nFILES YOU MAY WRITE: {}\nDEPENDENCIES SATISFIED: yes\n",
            task.id,
            task.title,
            task.description,
            task.acceptance.criteria,
            task.file_ownership.join(", "),
        );
        if let Some(fb) = feedback {
            body.push_str("\n--- PREVIOUS ATTEMPT FAILED SELF-CHECK ---\n");
            body.push_str(fb);
            body.push_str("\nFix the code and return the FULL corrected file set.\n");
        }
        InferenceRequest {
            task_type: "code".into(),
            system_prefix: Self::system_prefix(),
            context: input.context.clone(),
            input: body,
            max_tokens: Some(4096),
        }
    }
}

#[async_trait]
impl Agent for A5Coder {
    fn id(&self) -> &'static str {
        "A5"
    }
    fn title(&self) -> &'static str {
        "Lead Coder"
    }

    async fn run_task(&self, ctx: &AgentContext, input: &TaskInput) -> Result<ExecutionResult> {
        let started = crate::clock::now_utc_iso8601();
        #[allow(unused_assignments)]
        let mut artifacts: Vec<Artifact> = Vec::new();
        let mut commands: Vec<CommandRun> = Vec::new();
        let mut logs: Vec<LogEntry> = Vec::new();
        let mut turns_used = 0u32;
        let mut last_fail = String::new();
        // The LAST LLM response — on fix-loop exhaustion it carries the real
        // usage so the failed result still books an honest cost.
        let mut last_response: Option<crate::inference::InferenceResponse> = None;

        // §6.3 fix loop: initial turn + at most FIX_LOOP_MAX-1 corrections.
        for iteration in 0..FIX_LOOP_MAX {
            let req = Self::request(
                &input.task,
                input,
                if iteration == 0 {
                    None
                } else {
                    Some(&last_fail)
                },
            );
            let out = self
                .complete_json(ctx, "coding_task.schema.json", &req)
                .await?;
            turns_used += out.turns;
            let response = out.response.clone();
            let coding = Self::parse_output(&input.task.id, out.value)?;
            if !coding.notes.is_empty() && iteration == 0 {
                logs.push(LogEntry {
                    level: "info".into(),
                    message: format!("model notes: {}", coding.notes),
                    ts: crate::clock::now_utc_iso8601(),
                });
            }
            if iteration > 0 {
                logs.push(LogEntry {
                    level: "info".into(),
                    message: format!(
                        "fix loop iteration {iteration}: rewriting after failed self-check"
                    ),
                    ts: crate::clock::now_utc_iso8601(),
                });
            }

            artifacts = Self::write_patches(&ctx.workdir, &coding.patches)?;

            if coding.self_check.is_empty() {
                // No self-check declared: the gateway pass cannot be claimed.
                logs.push(LogEntry {
                    level: "warn".into(),
                    message:
                        "model declared no self_check commands; accepting on write-only evidence"
                            .into(),
                    ts: crate::clock::now_utc_iso8601(),
                });
                return Ok(Self::result(
                    &input.task,
                    ctx,
                    started,
                    "success",
                    format!(
                        "wrote {} file(s); NO verification run (model declared none)",
                        artifacts.len()
                    ),
                    artifacts,
                    commands,
                    logs,
                    &response,
                    turns_used,
                ));
            }

            let (ok, runs, fail) = Self::self_check(ctx, &coding.self_check);
            commands = runs;
            if ok {
                return Ok(Self::result(
                    &input.task,
                    ctx,
                    started,
                    "success",
                    format!(
                        "wrote {} file(s); self-check passed ({} command(s))",
                        artifacts.len(),
                        commands.len()
                    ),
                    artifacts,
                    commands,
                    logs,
                    &response,
                    turns_used,
                ));
            }
            last_response = Some(response);
            last_fail = fail;
            tracing::warn!(task = %input.task.id, iteration, "A5 self-check failed; fix loop continues");
        }

        // §8.1 E1: loop exhausted → the task FAILS as an ExecutionResult (the
        // control plane records TaskFailed and cascades blocked); a panic/Err
        // here would crash the driver instead of feeding the ladder (§8).
        let response = last_response.expect("at least one LLM turn ran");
        Ok(Self::result(
            &input.task,
            ctx,
            started,
            "failed",
            format!(
                "A5 fix loop exhausted after {FIX_LOOP_MAX} iterations; last failure: {last_fail}"
            ),
            artifacts,
            commands,
            logs,
            &response,
            turns_used,
        ))
    }
}

impl A5Coder {
    /// Assemble the §11.3 contract from what ACTUALLY happened.
    #[allow(clippy::too_many_arguments)]
    fn result(
        task: &TaskItem,
        ctx: &AgentContext,
        started: String,
        status: &str,
        summary: String,
        artifacts: Vec<Artifact>,
        commands: Vec<CommandRun>,
        logs: Vec<LogEntry>,
        resp: &crate::inference::InferenceResponse,
        turns: u32,
    ) -> ExecutionResult {
        let cost = estimate_cost(&resp.provider, resp.usage.tokens_in, resp.usage.tokens_out);
        ExecutionResult {
            schema_version: "1.0".into(),
            task_id: task.id.clone(),
            agent_id: self_id(ctx),
            started_at: started,
            finished_at: crate::clock::now_utc_iso8601(),
            status: status.into(),
            summary,
            artifacts,
            logs,
            model_usage: Some(ModelUsage {
                provider: resp.provider.clone(),
                model: resp.model.clone(),
                tokens_in: resp.usage.tokens_in,
                tokens_out: resp.usage.tokens_out,
                cached_tokens: resp.usage.cached_tokens,
                cost_usd: cost,
                cache_savings_usd: 0.0,
            }),
            toolchain_used: commands.first().map(|c| {
                c.cmd
                    .split_whitespace()
                    .next()
                    .unwrap_or("tool")
                    .to_string()
            }),
            commands_run: commands,
            next_agent: Some("A7".into()), // §6.3: code → security next
            error: None,
            escalate_to_ceo: false,
            human_approval_needed: false,
            idempotency: Some(crate::models::execution::Idempotency {
                op_id: format!("op-{}", hash::short_hex(task.id.as_bytes())),
                input_hash: hash::sha256_hex(format!("{}|{}", task.id, task.title).as_bytes()),
            }),
            extra: {
                let mut m = serde_json::Map::new();
                m.insert("llm_turns".into(), json!(turns));
                m
            },
        }
    }
}

fn self_id(ctx: &AgentContext) -> String {
    ctx.agent_id.clone()
}

/// Provider price table — §8.2 m1. Canonical home is `inference::estimate_cost`
/// (the pricing layer owns pricing); this delegate keeps the public path the
/// task agents already use (A4 bills through it too).
pub fn estimate_cost(provider: &str, tokens_in: u64, tokens_out: u64) -> f64 {
    crate::inference::estimate_cost(provider, tokens_in, tokens_out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_path_rejects_traversal_absolute_and_tests_dir() {
        let wd = Path::new("/tmp/wd");
        assert!(A5Coder::safe_path(wd, "src/main.rs").is_ok());
        assert!(A5Coder::safe_path(wd, "../evil.rs").is_err());
        assert!(A5Coder::safe_path(wd, "/etc/passwd").is_err());
        assert!(A5Coder::safe_path(wd, "tests/lib.rs").is_err(), "§5.9");
        assert!(
            A5Coder::safe_path(wd, "src/../tests/x.rs").is_err(),
            "hidden §5.9"
        );
        assert!(
            A5Coder::safe_path(wd, "src\\win.rs").is_err(),
            "no separators"
        );
    }

    #[test]
    fn parse_output_rejects_wrong_task_id_and_empty_files() {
        let doc = json!({
            "schema_version": "1.0", "task_id": "T9",
            "files": [{"path": "a.rs", "content": "x"}],
            "self_check": {"commands": []}
        });
        assert!(A5Coder::parse_output("T1", doc).is_err());
        let doc = json!({
            "schema_version": "1.0", "task_id": "T1",
            "files": [], "self_check": {"commands": []}
        });
        assert!(A5Coder::parse_output("T1", doc).is_err());
    }

    #[test]
    fn cost_table_covers_real_engines_and_zeroes_unknown() {
        assert!(estimate_cost("openrouter", 1_000_000, 1_000_000) > 0.0);
        assert_eq!(estimate_cost("local", 1_000_000, 1_000_000), 0.0);
        assert_eq!(estimate_cost("mock", 1_000_000, 1_000_000), 0.0);
    }
}
