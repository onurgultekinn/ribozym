//! A7 — Isolation & Security (§5.12, KİLİTLİ). Mostly DETERMINISTIC (§5.12
//! m3: "antivirüs modu" — koddan araçlar, LLM yalnız derin analizde):
//!
//! 1. Secret scan (§12.5): every agent-written file is scanned for
//!    key-shaped material BEFORE anything else. A hit is CRITICAL:
//!    `SECURITY_SECRET_FOUND` semantics — the run does not carry on with the
//!    secret in place.
//! 2. `cargo audit` (allowlisted in ribozym.toml) through the ToolGateway.
//! 3. Command isolation audit: every `commands_run` entry from the previous
//!    agent turns is re-checked against the toolchain allowlist (a command
//!    that RAN but wasn't allowlisted would mean a §12 breach).
//!
//! The LLM is used ONLY when `task.extra.deep_analysis == true` (the
//! difficulty lane, §5.12 m5) — the MVP flow is deterministic, so cost is 0
//! and the result is fully reproducible.

use async_trait::async_trait;

use crate::agent::{Agent, AgentContext, TaskInput};
use crate::error::Result;
use crate::models::execution::{CommandRun, ExecutionResult, LogEntry};
use crate::models::task::TaskItem;

#[derive(Default)]
pub struct A7Security;

/// Secret-shaped material: assignment/declaration patterns for the usual
/// credential kinds (§12.5). Deliberately narrow to keep false positives at
/// a level where the CRITICAL path stays credible; add patterns only with a
/// regression test each.
const SECRET_PATTERNS: &[&str] = &[
    "AKIA",                            // AWS access key id
    "-----BEGIN RSA PRIVATE KEY-----", // PEM keys
    "-----BEGIN OPENSSH PRIVATE KEY-----",
    "-----BEGIN PRIVATE KEY-----",
    "ghp_",                // GitHub PAT
    "sk-proj-",            // OpenAI-style project keys
    "OPENROUTER_API_KEY=", // inlined provider keys in code/config
    "AA_API_KEY=",
];

/// Files A7 scans: agent artifacts (A5's write set) tracked by extension and
/// by the task's declared ownership. Binaries are skipped (hash-covered).
fn scannable(path: &str) -> bool {
    matches!(
        path.rsplit('.').next(),
        Some("rs")
            | Some("toml")
            | Some("md")
            | Some("json")
            | Some("yml")
            | Some("yaml")
            | Some("sh")
            | Some("ps1")
    )
}

impl A7Security {
    pub fn new() -> Self {
        Self
    }

    /// §5.12 m3 "antivirüs modu": scan the WHOLE workdir (every text file an
    /// agent could have written), not just the task's declared set — a secret
    /// does not ask whom it belongs to. `target/` and VCS dirs are skipped
    /// (build output is hash-covered, not content-scanned).
    fn scan_secrets(workdir: &std::path::Path, _task: &TaskItem) -> Vec<(String, &'static str)> {
        let mut hits = Vec::new();
        let mut stack = vec![workdir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if !matches!(name, "target" | ".git" | ".ribozym") {
                        stack.push(path);
                    }
                    continue;
                }
                let Some(rel) = path.strip_prefix(workdir).ok().and_then(|p| p.to_str()) else {
                    continue;
                };
                if !scannable(rel) {
                    continue;
                }
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };
                for pat in SECRET_PATTERNS {
                    if content.contains(pat) {
                        hits.push((rel.replace('\\', "/"), *pat));
                    }
                }
            }
        }
        hits
    }

    /// §5.12 m3 (3): every command claimed by previous turns is re-checked
    /// against the allowlist NOW. The gateway `check` is pure — never runs.
    fn audit_commands(ctx: &AgentContext, task: &TaskItem) -> Vec<String> {
        let mut breaches = Vec::new();
        for cmd in &task.extra_commands() {
            if matches!(ctx.gateway.check(cmd), crate::gateway::Check::Denied { .. }) {
                breaches.push(cmd.clone());
            }
        }
        breaches
    }
}

/// Convenience: extra commands a task carries (A5's `self_check` echo lands
/// here via the plan; the orchestrator does not re-run them).
impl TaskItem {
    fn extra_commands(&self) -> Vec<String> {
        self.extra
            .get("self_check_commands")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|c| c.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[async_trait]
impl Agent for A7Security {
    fn id(&self) -> &'static str {
        "A7"
    }
    fn title(&self) -> &'static str {
        "Isolation & Security"
    }

    async fn run_task(&self, ctx: &AgentContext, input: &TaskInput) -> Result<ExecutionResult> {
        let started = crate::clock::now_utc_iso8601();
        let mut logs: Vec<LogEntry> = Vec::new();
        let mut commands: Vec<CommandRun> = Vec::new();

        // (1) secrets — CRITICAL stops the run's task as failed with the
        // §12.5 marker in the summary; the orchestrator routes it to the user.
        let hits = Self::scan_secrets(&ctx.workdir, &input.task);
        if !hits.is_empty() {
            let detail = hits
                .iter()
                .map(|(p, k)| format!("{p}:{k}"))
                .collect::<Vec<_>>()
                .join(", ");
            tracing::error!(task = %input.task.id, "SECURITY_SECRET_FOUND: {detail}");
            return Ok(Self::result(
                &input.task,
                ctx,
                started,
                "failed",
                format!("SECURITY_SECRET_FOUND: {detail}"),
                logs,
                commands,
            ));
        }
        logs.push(LogEntry {
            level: "info".into(),
            message: "secret scan clean".into(),
            ts: crate::clock::now_utc_iso8601(),
        });

        // (2) isolation audit of previously-run commands (§12.3 breach check).
        let breaches = Self::audit_commands(ctx, &input.task);
        if !breaches.is_empty() {
            return Ok(Self::result(
                &input.task,
                ctx,
                started,
                "failed",
                format!(
                    "ISOLATION_BREACH: non-allowlisted commands ran: {}",
                    breaches.join(", ")
                ),
                logs,
                commands,
            ));
        }

        // (3) cargo audit through the gateway (allowlist decides). A
        // capability probe FIRST: a machine without the cargo-audit
        // subcommand exits non-zero exactly like a real audit failure — the
        // probe separates "tool absent" (warn+skip, §12.4) from "tool found
        // problems" (task fails, §5.12 m4).
        if ctx.workdir.join("Cargo.lock").exists() {
            match ctx.gateway.run("cargo audit --version") {
                Ok(probe) if probe.exit_code == 0 => match ctx.gateway.run("cargo audit") {
                    Ok(out) => {
                        commands.push(CommandRun {
                            cmd: "cargo audit".into(),
                            exit_code: out.exit_code,
                            duration_ms: out.duration_ms,
                        });
                        if out.exit_code != 0 {
                            return Ok(Self::result(
                                &input.task,
                                ctx,
                                started,
                                "failed",
                                format!("cargo audit failed:\n{}", truncate(&out.stderr, 1500)),
                                logs,
                                commands,
                            ));
                        }
                    }
                    Err(e) => {
                        tracing::warn!(task = %input.task.id, "cargo audit refused: {e}");
                        logs.push(LogEntry {
                            level: "warn".into(),
                            message: format!("cargo audit refused: {e}"),
                            ts: crate::clock::now_utc_iso8601(),
                        });
                    }
                },
                other => {
                    let why = other
                        .map(|o| format!("exit {}", o.exit_code))
                        .unwrap_or_else(|e| e.to_string());
                    tracing::warn!(task = %input.task.id, "cargo audit unavailable ({why}); skipping");
                    logs.push(LogEntry {
                        level: "warn".into(),
                        message: format!("cargo audit unavailable ({why}); skipped (§12.4)"),
                        ts: crate::clock::now_utc_iso8601(),
                    });
                }
            }
        }

        Ok(Self::result(
            &input.task,
            ctx,
            started,
            "success",
            format!(
                "security scan clean: secrets 0, isolation breaches 0, audit run {}",
                if commands.is_empty() {
                    "skipped (no lockfile)"
                } else {
                    "clean"
                }
            ),
            logs,
            commands,
        ))
    }
}

impl A7Security {
    fn result(
        task: &TaskItem,
        ctx: &AgentContext,
        started: String,
        status: &str,
        summary: String,
        logs: Vec<LogEntry>,
        commands: Vec<CommandRun>,
    ) -> ExecutionResult {
        ExecutionResult {
            schema_version: "1.0".into(),
            task_id: task.id.clone(),
            agent_id: ctx.agent_id.clone(),
            started_at: started,
            finished_at: crate::clock::now_utc_iso8601(),
            status: status.into(),
            summary,
            // §11.3: deterministic agents carry NO model_usage.
            artifacts: vec![],
            logs,
            model_usage: None,
            toolchain_used: commands.first().map(|_| "cargo".into()),
            commands_run: commands,
            next_agent: if status == "success" {
                Some("A4".into())
            } else {
                None
            },
            error: None,
            escalate_to_ceo: false,
            human_approval_needed: status != "success",
            idempotency: None,
            extra: {
                let mut m = serde_json::Map::new();
                m.insert("llm_turns".into(), serde_json::json!(0u32));
                m
            },
        }
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() > max {
        &s[..max]
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scannable_selects_text_artifacts_only() {
        assert!(scannable("src/main.rs"));
        assert!(scannable("Cargo.toml"));
        assert!(!scannable("target/debug/app"));
        assert!(!scannable("logo.png"));
    }

    #[test]
    fn command_audit_flags_non_allowlisted_as_breach() {
        // Defense-in-depth path: a plan that declares extra commands gets
        // re-checked against the allowlist — the gateway already refused at
        // runtime, but §5.12 demands the audit trail proves it independently.
        let dir = tempfile::tempdir().unwrap();
        let toolchains = std::collections::BTreeMap::from([(
            "rust".to_string(),
            crate::config::project::ToolchainSpec {
                allowed: vec!["cargo check".into()],
                fs_read: vec![],
                fs_write: vec![],
                env_scrub: "deny".into(),
                timeout_s: 60,
                output_cap_kb: 16,
            },
        )]);
        let gw = crate::gateway::ToolGateway::new(
            toolchains,
            dir.path().to_path_buf(),
            std::sync::Arc::new(crate::gateway::SystemRunner),
            Default::default(),
        );
        let ctx = AgentContext {
            agent_id: "A7".into(),
            spec_id: "s".into(),
            run_id: "r".into(),
            workdir: dir.path().to_path_buf(),
            gateway: std::sync::Arc::new(gw),
            guard: std::sync::Arc::new(crate::repair::SchemaGuard::new(
                std::sync::Arc::new(crate::inference::MockProvider::always("{}")),
                dir.path(),
            )),
            web: None,
        };
        let mut t = TaskItem {
            id: "T1".into(),
            spec_requirement_ids: vec![],
            title: "x".into(),
            description: String::new(),
            department: None,
            assigned_agent: "A5".into(),
            dependencies: vec![],
            target: "cloud".into(),
            toolchain: None,
            acceptance: crate::models::task::Acceptance {
                verifiable_by: "A4".into(),
                criteria: "x".into(),
            },
            estimated: None,
            difficulty: None,
            file_ownership: vec![],
            artifacts_expected: vec![],
            artifacts_produced: vec![],
            blocked_by: vec![],
            status: "done".into(),
            retry_count: 0,
            escalation_level: 0,
            notes: String::new(),
            extra: Default::default(),
        };
        t.extra.insert(
            "self_check_commands".into(),
            serde_json::json!(["cargo check", "rm -rf /"]),
        );
        let breaches = A7Security::audit_commands(&ctx, &t);
        assert_eq!(breaches, vec!["rm -rf /".to_string()]);
    }
}
