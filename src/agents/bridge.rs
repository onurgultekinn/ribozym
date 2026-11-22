//! The Faz-5 seam: sync `TaskExecutor` (control plane) ↔ async `Agent`
//! (§5.3). See `agents/mod.rs` for the rationale.
//!
//! Also owns AgentContext construction from a `RunConfig`-flavored set of
//! inputs: provider (SchemaGuard-wrapped), gateway, workdir.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::agent::{AgentContext, TaskInput};
use crate::error::{Error, Result};
use crate::gateway::ToolGateway;
use crate::inference::InferenceProvider;
use crate::models::task::TaskItem;
use crate::orchestrator::{Executed, TaskExecutor};
use crate::repair::SchemaGuard;

/// Executes tasks by dispatching them to the agent named in
/// `task.assigned_agent`. Deterministic per (task, provider-script): the
/// control plane's golden digest remains stable.
pub struct AgentBridge {
    /// Schema dir (repo root) for the SchemaGuard.
    schema_dir: PathBuf,
    provider: Arc<dyn InferenceProvider>,
    gateway: Arc<ToolGateway>,
    /// Isolated working copy the agents write into (§12.6).
    workdir: PathBuf,
    /// Run-scoped context handed to every agent (context.md stand-in until
    /// A1's Researching output exists — honest, labeled).
    context: String,
    /// The §13 web door; None until the composition root attaches one.
    web: Option<std::sync::Arc<crate::web::WebDoor>>,
}

impl AgentBridge {
    pub fn new(
        schema_dir: impl Into<PathBuf>,
        provider: Arc<dyn InferenceProvider>,
        gateway: Arc<ToolGateway>,
        workdir: impl Into<PathBuf>,
        context: impl Into<String>,
    ) -> Self {
        Self {
            schema_dir: schema_dir.into(),
            provider,
            gateway,
            workdir: workdir.into(),
            context: context.into(),
            web: None,
        }
    }

    /// Attach the web door (§13) — the builder keeps every existing
    /// construction site valid (no web = honest absence, A11 fails loudly).
    pub fn with_web(mut self, door: std::sync::Arc<crate::web::WebDoor>) -> Self {
        self.web = Some(door);
        self
    }

    fn context_for(&self, task: &TaskItem) -> Result<AgentContext> {
        Ok(AgentContext {
            agent_id: task.assigned_agent.clone(),
            spec_id: String::new(), // stamped by the caller when known
            run_id: String::new(),
            workdir: self.workdir.clone(),
            gateway: Arc::clone(&self.gateway),
            guard: Arc::new(SchemaGuard::new(
                Arc::clone(&self.provider),
                self.schema_dir.clone(),
            )),
            web: self.web.clone(),
        })
    }
}

impl TaskExecutor for AgentBridge {
    fn execute(&self, task: &TaskItem, _op_id: &str) -> Result<Executed> {
        let agent = crate::agents::lookup(&task.assigned_agent)?;
        let ctx = self.context_for(task)?;
        let input = TaskInput {
            task: task.clone(),
            // §10.4: role prefix is byte-stable; task data lives in input.
            system_prefix: String::new(),
            context: self.context.clone(),
        };
        // One current-thread runtime per task: no shared reactor, so agent
        // scheduling cannot perturb control-plane determinism.
        // §8.1: a semantic agent failure (untraceable test, wrong task_id,
        // provider refusal) is the LADDER's input, not a driver crash — it
        // converts to a failed Executed so the control plane records
        // TaskFailed and cascades blocked (§6.2), exactly like a failed run.
        let result = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| Error::Agent(format!("runtime: {e}")))?
            .block_on(agent.run_task(&ctx, &input))
        {
            Ok(r) => r,
            Err(e) => {
                // §8.2/B3: a money refusal at the call boundary is NOT a
                // task defect. The driver pauses the run (keeping completed
                // work); the ladder is for broken work, not empty wallets.
                let budget_denied = matches!(&e, Error::ModelBudgetDenied(_));
                tracing::error!(
                    task = %task.id,
                    agent = %task.assigned_agent,
                    budget_denied,
                    "agent turn failed: {e}"
                );
                return Ok(Executed {
                    summary: serde_json::json!({
                        "task": task.id,
                        "agent": task.assigned_agent,
                        "error": e.to_string(),
                    }),
                    ok: false,
                    cost_usd: 0.0,
                    budget_denied,
                });
            }
        };

        if result.status != "success" {
            return Ok(Executed {
                summary: serde_json::json!({
                    "task": task.id,
                    "status": result.status,
                    "summary": result.summary,
                }),
                ok: false,
                cost_usd: result
                    .model_usage
                    .as_ref()
                    .map(|u| u.cost_usd)
                    .unwrap_or(0.0),
                budget_denied: false,
            });
        }
        Ok(Executed {
            summary: serde_json::json!({
                "task": task.id,
                "agent": result.agent_id,
                "summary": result.summary,
                "artifacts": result.artifacts.iter().map(|a| a.path.clone()).collect::<Vec<_>>(),
                "llm_turns": result.extra.get("llm_turns").cloned().unwrap_or(serde_json::json!(0)),
                // A3's report consumes these from the TaskCompleted event
                // stream (§5.7: numbers from the system of record) — the
                // honest per-turn accounting, not a reconstructed guess.
                "model": result.model_usage.as_ref().map(|u| u.model.clone()),
                "provider": result.model_usage.as_ref().map(|u| u.provider.clone()),
                "tokens": result.model_usage.as_ref().map(|u| serde_json::json!({
                    "in": u.tokens_in, "out": u.tokens_out, "cached": u.cached_tokens
                })),
                "cost_usd": result.model_usage.as_ref().map(|u| u.cost_usd),
                "cache_savings_usd": result
                    .model_usage
                    .as_ref()
                    .map(|u| u.cache_savings_usd),
            }),
            ok: true,
            cost_usd: result
                .model_usage
                .as_ref()
                .map(|u| u.cost_usd)
                .unwrap_or(0.0),
            budget_denied: false,
        })
    }
}

/// Sanity: the workdir must exist before agents write into it.
pub fn ensure_workdir(p: &Path) -> Result<()> {
    std::fs::create_dir_all(p)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::registry;

    #[test]
    fn registry_carries_a5_and_refuses_unknowns() {
        let r = registry();
        assert!(r.contains_key("A5"), "MVP starts with the coder");
        let msg = match crate::agents::lookup("A6") {
            Err(e) => e.to_string(),
            Ok(_) => panic!("A6 must not be in the MVP registry"),
        };
        assert!(msg.contains("MVP registry"), "{msg}");
        assert!(crate::agents::lookup("A5").is_ok());
    }

    #[test]
    fn ensure_workdir_creates_missing_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        ensure_workdir(&nested).unwrap();
        assert!(nested.is_dir());
    }
}
