//! Agent interface (Faz 3, §5, §6.2). A concrete agent = one A1–A12 role
//! implementation that turns a `TaskItem` into an `ExecutionResult` through
//! exactly two capabilities, both injected via `AgentContext`:
//! * LLM turns — schema-forced (`SchemaGuard`: one repair turn, §14);
//! * tools — allowlisted (`ToolGateway`, §12).
//!
//! Everything else (budget, events, retries, escalation) belongs to the
//! control plane (Phase 4), NOT to agents — ownership discipline §6.2.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::error::Result;
use crate::gateway::ToolGateway;
use crate::inference::InferenceRequest;
use crate::models::execution::ExecutionResult;
use crate::models::task::TaskItem;
use crate::repair::{GuardedOutput, SchemaGuard};
use crate::web::WebDoor;

/// Everything an agent may touch. Constructed by the orchestrator per task.
pub struct AgentContext {
    pub agent_id: String,
    pub spec_id: String,
    pub run_id: String,
    /// Isolated working copy of the target project (§12.6 — the original
    /// project is NEVER written directly).
    pub workdir: PathBuf,
    pub gateway: Arc<ToolGateway>,
    pub guard: Arc<SchemaGuard>,
    /// §13: A11's only-content door (backend + cache + rule verifier).
    /// None means "no web in this run" — A11 then fails visibly.
    pub web: Option<Arc<WebDoor>>,
}

/// One dispatched task: the DAG node plus the prompt material for it.
pub struct TaskInput {
    pub task: TaskItem,
    /// Byte-stable system prefix for this agent role (§10.4 caching).
    pub system_prefix: String,
    /// Run-scoped context (archive section, memory summary).
    pub context: String,
}

#[async_trait]
pub trait Agent: Send + Sync {
    /// "A1".."A12" — stable registry id (agents.toml).
    fn id(&self) -> &'static str;

    /// Human-readable role title, mirrors agents.toml.
    fn title(&self) -> &'static str;

    /// Execute one task. Implementations produce an ExecutionResult; failure
    /// semantics (retries, escalation) stay with the control plane.
    async fn run_task(&self, ctx: &AgentContext, input: &TaskInput) -> Result<ExecutionResult>;

    /// Shared helper: one schema-forced LLM turn (repair loop included).
    /// Concrete agents call this instead of touching providers directly —
    /// §3.1: providers never self-approve.
    async fn complete_json(
        &self,
        ctx: &AgentContext,
        schema_name: &str,
        request: &InferenceRequest,
    ) -> Result<GuardedOutput> {
        ctx.guard.complete(request, schema_name).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::MockProvider;
    use crate::models::task::Acceptance;
    use std::collections::BTreeMap;

    struct EchoAgent;

    #[async_trait]
    impl Agent for EchoAgent {
        fn id(&self) -> &'static str {
            "A5"
        }
        fn title(&self) -> &'static str {
            "Codesmith"
        }
        async fn run_task(&self, ctx: &AgentContext, input: &TaskInput) -> Result<ExecutionResult> {
            // Turn the schema-forced LLM output into a result contract.
            let req = InferenceRequest {
                task_type: "code".into(),
                system_prefix: input.system_prefix.clone(),
                context: input.context.clone(),
                input: input.task.title.clone(),
                max_tokens: Some(512),
            };
            let out = self
                .complete_json(ctx, "execution_result.schema.json", &req)
                .await?;
            let mut result: ExecutionResult = serde_json::from_value(out.value)?;
            result.agent_id = self.id().to_string();
            result.task_id = input.task.id.clone();
            Ok(result)
        }
    }

    fn fixture_ctx() -> (AgentContext, Arc<MockProvider>) {
        let mock = Arc::new(MockProvider::always(
            r#"{"schema_version":"1.0","task_id":"T1","agent_id":"A5","started_at":"2026-09-12T10:00:00Z","finished_at":"2026-09-12T10:01:00Z","status":"success","summary":"did the thing"}"#,
        ));
        let guard = Arc::new(SchemaGuard::new(
            mock.clone(),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        ));
        let gateway = Arc::new(ToolGateway::new(
            BTreeMap::new(),
            PathBuf::from("."),
            Arc::new(crate::gateway::SystemRunner),
            BTreeMap::new(),
        ));
        (
            AgentContext {
                agent_id: "A5".into(),
                spec_id: "spec-20260912-001".into(),
                run_id: "run-x1".into(),
                workdir: PathBuf::from("."),
                gateway,
                guard,
                web: None,
            },
            mock,
        )
    }

    fn task() -> TaskItem {
        TaskItem {
            id: "T1".into(),
            spec_requirement_ids: vec!["R1".into()],
            title: "write module".into(),
            description: String::new(),
            department: None,
            assigned_agent: "A5".into(),
            dependencies: vec![],
            target: "src/lib.rs".into(),
            toolchain: None,
            acceptance: Acceptance {
                verifiable_by: "test".into(),
                criteria: "tests pass".into(),
            },
            estimated: None,
            difficulty: None,
            file_ownership: vec![],
            artifacts_expected: vec![],
            artifacts_produced: vec![],
            blocked_by: vec![],
            status: "pending".into(),
            retry_count: 0,
            escalation_level: 0,
            notes: String::new(),
            extra: Default::default(),
        }
    }

    #[tokio::test]
    async fn agent_runs_schema_forced_turn_into_execution_result() {
        let (ctx, mock) = fixture_ctx();
        let agent = EchoAgent;
        let input = TaskInput {
            task: task(),
            system_prefix: "SYSTEM (fixed prefix)".into(),
            context: "session context".into(),
        };
        let result = agent.run_task(&ctx, &input).await.unwrap();
        assert_eq!(result.task_id, "T1");
        assert_eq!(result.agent_id, "A5");
        assert_eq!(result.status, "success");
        assert_eq!(mock.calls.lock().unwrap().len(), 1);
        assert_eq!(agent.id(), "A5");
        assert_eq!(agent.title(), "Codesmith");
    }
}
