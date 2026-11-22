//! Binary entry — maps CLI results to §20.2 exit codes and prints structured
//! output. All logic lives in the library (P1).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use ribozym::cli::{BudgetCommand, Cli, Command, ConfigCommand, SchemaCommand};
use ribozym::config;
use ribozym::gates::Gate;
use ribozym::hash;
use ribozym::models::task::TaskPlan;
use ribozym::orchestrator::{MockExecutor, Orchestrator, Outcome, PlanningOutcome, RunConfig};
use ribozym::pipeline::AgentPipeline;
use ribozym::validate;

// §20.2 exit-code table — named once, every arm returns a code, never magic.
mod codes {
    pub const OK: u8 = 0;
    pub const USER_ERROR: u8 = 1;
    pub const NEEDS_USER: u8 = 2;
    /// §8.2/B3: a run that halted at the budget wall (Outcome::Paused) exits
    /// with this; raising the budget and re-running resumes from paused_from.
    pub const BUDGET: u8 = 3;
    pub const APPROVAL: u8 = 4;
    pub const CRASH: u8 = 5;
}

fn main() -> ExitCode {
    // §15: WARN+ telemetry (rule-4 skips, local→cloud fallbacks, stale-lock
    // recovery) goes to stderr. events.jsonl is the RUN's record and never
    // mixes with the process log — the two surfaces stay separate on purpose.
    let _ = ribozym::logging::LineLogger::stderr().install();
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            // ConfigInvalid and §6.4 Traceability are user-correctable
            // (§20.2 code 1); anything else is a control-plane crash (5).
            let code = match &e {
                ribozym::Error::ConfigInvalid(_) | ribozym::Error::Traceability(_) => {
                    codes::USER_ERROR
                }
                _ => codes::CRASH,
            };
            eprintln!("error: {e}");
            ExitCode::from(code)
        }
    }
}

/// Outcome → §20.2 code. `Delivered` is the only code-0 success.
/// - `Waiting` (gate open) → 4: the approval channel; a CI must not read
///   "still needs a human" as "succeeded".
/// - `Paused` (budget wall) → 3: BUDGET-family.
/// - `NeedsUser` AND `Failed` → 2: §20.2 has no separate "run failed cleanly"
///   code; both mean "the run stopped and a human must act" (the printed
///   `why` distinguishes a gate veto from a dead DAG). NOT 1 — a run that
///   executed with valid arguments and failed is not a user/argument error.
fn outcome_code(out: &Outcome) -> u8 {
    match out {
        Outcome::Delivered => codes::OK,
        Outcome::Waiting { .. } => codes::APPROVAL,
        Outcome::Paused { .. } => codes::BUDGET,
        Outcome::NeedsUser { .. } | Outcome::Failed { .. } => codes::NEEDS_USER,
    }
}

fn run(command: Command) -> ribozym::Result<u8> {
    match command {
        Command::Config {
            command: ConfigCommand::Validate,
        } => {
            let root = require_root()?;
            let problems = validate::validate_config(&root)?;
            if problems.is_empty() {
                println!("config validate: OK");
                Ok(codes::OK)
            } else {
                for p in problems {
                    println!("INVALID: {p}");
                }
                Ok(codes::USER_ERROR)
            }
        }
        Command::Schema {
            command: SchemaCommand::Validate { schema, file, full },
        } => {
            let doc = load_json::<serde_json::Value>(&file, "document")?;
            let problems = if full {
                validate::validate_full(&require_root()?, &schema, &doc)
            } else {
                validate::validate_document(&schema, &doc)
            };
            if problems.is_empty() {
                println!("{file}: OK against {schema}");
                Ok(codes::OK)
            } else {
                for p in problems {
                    println!("INVALID: {p}");
                }
                Ok(codes::USER_ERROR)
            }
        }
        Command::Run {
            mode,
            tasks,
            spec,
            form,
            spec_id,
            workdir,
            yes,
            autonomy,
            assume_compliant,
        } => match mode.as_str() {
            "mini" => run_mini(
                tasks.as_deref(),
                spec.as_deref(),
                yes,
                autonomy,
                assume_compliant,
            ),
            "interactive" => run_interactive(
                form.as_deref(),
                spec_id.as_deref(),
                workdir.as_deref(),
                yes,
                autonomy,
            ),
            other => {
                eprintln!("run: unknown mode {other:?} (mini | interactive)");
                Ok(codes::USER_ERROR)
            }
        },
        Command::Approve {
            spec_id,
            gate,
            note,
            deny,
            decided_by,
        } => approve(&spec_id, &gate, note.as_deref(), deny, &decided_by),
        Command::Status { spec_id } => status_surface(spec_id.as_deref()),
        Command::Report { spec_id, format } => report_surface(&spec_id, &format),
        Command::Budget {
            command: BudgetCommand::Show,
        } => budget_surface_cmd(),
        Command::NewForm { out } => {
            let path = ribozym::surface::new_form(std::path::Path::new(&out))?;
            println!(
                "{} written — fill it, then run `ribozym run --mode interactive --form {}`",
                path.display(),
                out
            );
            Ok(codes::OK)
        }
    }
}

/// `ribozym status [spec_id]`: the derived projection, never a re-computation
/// of its own numbers (§7.1 — the surfaces READ what the ledger says).
fn status_surface(spec_id: Option<&str>) -> ribozym::Result<u8> {
    let root = require_root()?;
    let runs = ribozym::surface::status_all(&root)?;
    let runs: Vec<_> = match spec_id {
        Some(id) => runs.into_iter().filter(|r| r.spec_id == id).collect(),
        None => runs,
    };
    if runs.is_empty() {
        match spec_id {
            Some(id) => {
                eprintln!("no run for spec {id:?} in this project");
                return Ok(codes::USER_ERROR);
            }
            None => {
                println!("no runs yet — start one with `ribozym run --mode interactive --form <form.md>`");
                return Ok(codes::OK);
            }
        }
    }
    for r in runs {
        let tasks: Vec<String> = r.tasks.iter().map(|(k, v)| format!("{k}={v}")).collect();
        println!(
            "{} [{}] {} | spent ${:.4}/${:.2} | tasks: {}{}",
            r.spec_id,
            r.run_id,
            r.phase,
            r.spent_usd,
            r.budget_usd,
            if tasks.is_empty() {
                "none".into()
            } else {
                tasks.join(" ")
            },
            r.pending_gate
                .as_ref()
                .map(|g| format!(" | GATE WAITS: {g}"))
                .unwrap_or_default(),
        );
        if let Some(reason) = &r.paused_reason {
            println!("    paused ({reason}) — raise budget and re-run to resume");
        }
        if !r.archive_corrupt.is_empty() {
            println!(
                "    ARCHIVE CORRUPT (§7.6 B2): {} — repair from the run documents or the run must park",
                r.archive_corrupt.join(", ")
            );
        }
    }
    Ok(codes::OK)
}

/// `ribozym report`: prints the SEALED artifact verbatim (json) or a
/// deterministic markdown rendering of it (md) — no new content is authored.
fn report_surface(spec_id: &str, format: &str) -> ribozym::Result<u8> {
    let root = require_root()?;
    let doc = ribozym::surface::report_json(&root, spec_id)?;
    match format {
        "json" => println!("{}", ribozym::hash::canonical_json(&doc)),
        "md" => print!("{}", ribozym::surface::render_md(&doc)),
        other => {
            eprintln!("report: unknown format {other:?} (md | json)");
            return Ok(codes::USER_ERROR);
        }
    }
    Ok(codes::OK)
}

fn budget_surface_cmd() -> ribozym::Result<u8> {
    let root = require_root()?;
    let pc = config::load_project_config(&root)?;
    let b = ribozym::surface::budget_surface(
        &root,
        pc.budget.monthly_usd,
        pc.budget.soft_limit_pct,
        pc.budget.hard_limit_pct,
    )?;
    println!(
        "frame: ${:.2}/month (soft {:.0}% · hard {:.0}%) — config-owned; ribozym never raises it",
        b.monthly_usd, b.soft_pct, b.hard_pct
    );
    if b.runs.is_empty() {
        println!("  no runs yet");
    }
    for (spec, status, spent, budget) in &b.runs {
        println!("  {spec}: ${spent:.4} / ${budget:.2} ({status})");
    }
    println!("total spent across runs: ${:.4}", b.total_spent);
    Ok(codes::OK)
}

fn require_root() -> ribozym::Result<PathBuf> {
    config::find_repo_root(&std::env::current_dir().unwrap_or_default()).ok_or_else(|| {
        ribozym::Error::ConfigInvalid("ribozym.toml not found (run inside the repo)".into())
    })
}

/// Read + parse a USER-SUPPLIED file, surfacing any failure (missing file,
/// bad JSON, wrong shape) as `ConfigInvalid` ⇒ §20.2 exit 1. A malformed input
/// document is a user error, never an "unexpected crash" (5) — the previous
/// `?`-propagation mislabelled `Error::Json` as a crash.
fn load_json<T: serde::de::DeserializeOwned>(path: &str, what: &str) -> ribozym::Result<T> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| ribozym::Error::ConfigInvalid(format!("cannot read {what} '{path}': {e}")))?;
    serde_json::from_str(&raw)
        .map_err(|e| ribozym::Error::ConfigInvalid(format!("{what} '{path}' is not valid: {e}")))
}

/// `ribozym approve` (§9 file-based/remote vote): writes the approvals row;
/// the waiting run picks it up on its next `run`/`resume` pass — approval
/// never races the driver, which keeps this a single-writer system (§7.5).
fn approve(
    spec_id: &str,
    gate: &str,
    note: Option<&str>,
    deny: bool,
    decided_by: &str,
) -> ribozym::Result<u8> {
    let root = require_root()?;
    let gate = Gate::parse(gate).ok_or_else(|| {
        ribozym::Error::ConfigInvalid(format!(
            "unknown gate {gate:?} (spec | task_plan | delivery)"
        ))
    })?;
    if !root.join(".ribozym").join("state.db").exists() {
        return Err(ribozym::Error::ConfigInvalid(
            "no state.db yet — start a run first (nothing to approve)".into(),
        ));
    }
    let store = ribozym::state::StateStore::open(&root.join(".ribozym").join("state.db"))?;
    // Guard against an ORPHAN vote: no run row means this spec was never
    // started, so approving would silently write a row no driver reads (and a
    // typo'd spec_id would look like it worked). Reject with exit 1.
    let known: i64 = store.conn().query_row(
        "SELECT COUNT(*) FROM runs WHERE spec_id = ?1",
        rusqlite::params![spec_id],
        |r| r.get(0),
    )?;
    if known == 0 {
        return Err(ribozym::Error::ConfigInvalid(format!(
            "no run for spec {spec_id:?} — nothing to approve (start it with `ribozym run`)"
        )));
    }
    let a = ribozym::gates::Approvals::new(store.conn());
    a.record(
        spec_id,
        gate,
        decided_by,
        if deny { "denied" } else { "approved" },
        note,
        &ribozym::clock::now_utc_iso8601(),
    )?;
    println!(
        "approved {} for {spec_id} as {decided_by} ({}).",
        gate.as_str(),
        if deny { "DENIAL" } else { "granted" }
    );
    Ok(if deny { codes::APPROVAL } else { codes::OK })
}

/// The deterministic Faz-4 path: a tasks.json through FSM + DAG + ledger +
/// gates. LLM never runs here (§21.1); Faz 5 replaces MockExecutor.
fn run_mini(
    tasks: Option<&str>,
    spec: Option<&str>,
    yes: bool,
    autonomy: Option<u8>,
    assume_compliant: bool,
) -> ribozym::Result<u8> {
    let root = require_root()?;
    let path = tasks.ok_or_else(|| {
        ribozym::Error::ConfigInvalid("run --mode mini needs --tasks <tasks.json>".into())
    })?;
    // Canonical schema validation (full jsonschema, not just the structural
    // subset) so nested-field errors are reported as INVALID + exit 1 rather
    // than surfacing later as a deserialization crash.
    let doc = load_json::<serde_json::Value>(path, "tasks document")?;
    let problems = validate::validate_full(&root, "tasks.schema.json", &doc);
    if !problems.is_empty() {
        for p in problems {
            println!("INVALID: {p}");
        }
        return Ok(codes::USER_ERROR);
    }
    let plan: TaskPlan = load_json(path, "tasks document")?;

    // §6.4 cross-doc traceability: only enforceable when the spec is supplied.
    // Absent ⇒ report the skip loudly; a silent pass would be a fake gate.
    let spec_model = match spec {
        Some(sp) => {
            let sdoc = load_json::<serde_json::Value>(sp, "spec document")?;
            let sprob = validate::validate_full(&root, "spec.schema.json", &sdoc);
            if !sprob.is_empty() {
                for p in sprob {
                    println!("INVALID (spec): {p}");
                }
                return Ok(codes::USER_ERROR);
            }
            Some(load_json::<ribozym::models::spec::Spec>(
                sp,
                "spec document",
            )?)
        }
        None => None,
    };

    let pc = config::load_project_config(&root)?;
    let now = ribozym::clock::now_epoch_seconds();
    let mut cfg = RunConfig {
        root: root.clone(),
        spec_id: plan.spec_id.clone(),
        run_id: format!(
            "run-{}",
            hash::short_hex(format!("{now}-{}", std::process::id()).as_bytes())
        ),
        autonomy_level: pc.run.default_autonomy_level,
        batch_yes: yes,
        max_concurrency: pc.run.max_concurrency,
        crisis_max: pc.errors.final_review_crisis_max,
        gate_timeout_seconds: pc.run.gate_timeout_seconds,
        budget_usd: pc.budget.monthly_usd,
        budget_soft_pct: pc.budget.soft_limit_pct,
        budget_hard_pct: pc.budget.hard_limit_pct,
        now_epoch: now,
    };
    // §9: the CLI flag may only LOWER autonomy (safe direction), never raise.
    if let Some(want) = autonomy {
        cfg.autonomy_level = cfg.autonomy_level.min(want);
    }

    // Single-copy discipline (§7.5): take the lock before touching state.db.
    let _lock = match ribozym::lock::RunLock::acquire(
        &root.join(".ribozym"),
        std::time::Duration::from_secs(30),
    ) {
        Ok((l, true)) => {
            eprintln!("warn: recovered stale lock from a crashed run");
            l
        }
        Ok((l, false)) => l,
        Err(ribozym::lock::AcquireError::HeldBy { pid, age }) => {
            eprintln!(
                "another ribozym copy holds the lock (pid {pid}, {}s) — refusing to run",
                age.as_secs()
            );
            return Ok(codes::USER_ERROR);
        }
        Err(ribozym::lock::AcquireError::Io(e)) => {
            return Err(ribozym::Error::ConfigInvalid(format!("lock: {e}")))
        }
    };

    let resumed = Orchestrator::exists(&cfg)?;
    let mut o = if resumed {
        let mut o = Orchestrator::resume(&cfg, &plan)?;
        // A paused run resumes only if the (possibly raised) budget now clears
        // the wall; advance() re-pauses otherwise — idempotent, no silent pass.
        if o.unpause()? {
            eprintln!("resuming a paused run under the current budget");
        }
        o
    } else {
        Orchestrator::create_with_spec(&cfg, &plan, spec_model.as_ref())?
    };
    let out = o.advance(&MockExecutor, assume_compliant)?;
    let digest = o.final_digest()?;
    println!("run_id: {}", cfg.run_id);
    println!(
        "traceability: {}",
        match &spec_model {
            Some(_) => "§6.4 enforced",
            None => "SKIPPED (no --spec)",
        }
    );
    println!("phase:    {}", phase_of(&out));
    match &out {
        Outcome::Delivered => println!(
            "spent_usd: {:.4} / budget {:.4}",
            o.spent_usd(),
            cfg.budget_usd
        ),
        Outcome::Waiting { gate } => println!(
            "gate {} waits — vote with `ribozym approve {} {}`, then `ribozym run` again",
            gate.as_str(),
            plan.spec_id,
            gate.as_str()
        ),
        Outcome::Paused { reason } => println!(
            "budget wall ({reason:?}): completed work kept, run parked — raise budget and re-run"
        ),
        Outcome::NeedsUser { why } => println!("needs user: {why}"),
        Outcome::Failed { why } => println!("failed: {why}"),
    }
    println!("final hash: {digest}");
    Ok(outcome_code(&out))
}

/// `ribozym run --mode interactive --form <spec-form.md>`: the full §6.1
/// management line + execution with the REAL provider stack. The lifecycle
/// itself is the one tests/planning_e2e.rs pins; this wrapper only builds
/// the world around it (provider, gateway, lock) and maps outcomes to the
/// §20.2 exit codes like `run_mini` does.
fn run_interactive(
    form: Option<&str>,
    spec_id: Option<&str>,
    workdir: Option<&str>,
    yes: bool,
    autonomy: Option<u8>,
) -> ribozym::Result<u8> {
    let root = require_root()?;
    let form_path = form.ok_or_else(|| {
        ribozym::Error::ConfigInvalid("run --mode interactive needs --form <spec-form.md>".into())
    })?;
    let form_bytes = std::fs::read(form_path).map_err(|e| {
        ribozym::Error::ConfigInvalid(format!("cannot read spec form \'{form_path}\': {e}"))
    })?;
    let problems = validate::check_form(&form_bytes);
    if !problems.is_empty() {
        // Fail fast at the CLI door with the user-facing reasons; the driver
        // would park identically, but the human is here NOW.
        for pr in problems {
            println!("INVALID FORM: {pr}");
        }
        return Ok(codes::USER_ERROR);
    }
    let workdir = PathBuf::from(workdir.ok_or_else(|| {
        ribozym::Error::ConfigInvalid(
            "run --mode interactive needs --workdir <isolated path> (§12.6)".into(),
        )
    })?);
    ribozym::agents::bridge::ensure_workdir(&workdir)?;

    // Spec id: the caller\'s, or derived from form date + content (a form is
    // its own identity — same bytes, same id, resumable).
    let spec_id = match spec_id {
        Some(v) => v.to_string(),
        None => {
            let hex = hash::short_hex(&form_bytes);
            let n = u32::from_str_radix(&hex[..3], 16).unwrap_or(0) % 1000;
            let date = ribozym::clock::now_utc_iso8601();
            format!("spec-{}-{:03}", date[..10].replace('-', ""), n)
        }
    };

    let pc = config::load_project_config(&root)?;
    let models = config::load_models(&root)?;
    let now = ribozym::clock::now_epoch_seconds();

    // Single-copy discipline (§7.5) before any state touch.
    let _lock = match ribozym::lock::RunLock::acquire(
        &root.join(".ribozym"),
        std::time::Duration::from_secs(30),
    ) {
        Ok((l, true)) => {
            eprintln!("warn: recovered stale lock from a crashed run");
            l
        }
        Ok((l, false)) => l,
        Err(ribozym::lock::AcquireError::HeldBy { pid, age }) => {
            eprintln!(
                "another ribozym copy holds the lock (pid {pid}, {}s) — refusing",
                age.as_secs()
            );
            return Ok(codes::USER_ERROR);
        }
        Err(ribozym::lock::AcquireError::Io(e)) => {
            return Err(ribozym::Error::ConfigInvalid(format!("lock: {e}")))
        }
    };

    // Resume keeps the run\'s OWN budget wall (§8.2: the row is the truth).
    let db = root.join(".ribozym").join("state.db");
    let existing: Option<(String, f64, f64)> = if db.exists() {
        let store = ribozym::state::StateStore::open(&db)?;
        store
            .conn()
            .query_row(
                "SELECT status, budget_usd, spent_usd FROM runs WHERE spec_id = ?1",
                rusqlite::params![spec_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok()
    } else {
        None
    };
    let cfg = RunConfig {
        root: root.clone(),
        spec_id: spec_id.clone(),
        run_id: format!(
            "run-{}",
            hash::short_hex(format!("{now}-{spec_id}").as_bytes())
        ),
        autonomy_level: pc.run.default_autonomy_level,
        batch_yes: yes,
        max_concurrency: pc.run.max_concurrency,
        crisis_max: pc.errors.final_review_crisis_max,
        gate_timeout_seconds: pc.run.gate_timeout_seconds,
        budget_usd: existing
            .as_ref()
            .map(|(_, b, _)| *b)
            .unwrap_or(pc.budget.monthly_usd),
        budget_soft_pct: pc.budget.soft_limit_pct,
        budget_hard_pct: pc.budget.hard_limit_pct,
        now_epoch: now,
    };
    let mut cfg = cfg;
    if let Some(want) = autonomy {
        cfg.autonomy_level = cfg.autonomy_level.min(want);
    }

    // The provider stack: cloud is the primary engine (Q1); local joins as
    // the grunt-work offload when the whitelist is non-empty (§10.1/Q1).
    let api_key = std::env::var(&models.openrouter.api_key_env).map_err(|_| {
        ribozym::Error::ConfigInvalid(format!(
            "{} not set — the cloud engine has no credentials",
            models.openrouter.api_key_env
        ))
    })?;
    // Provisional model pick until A10\'s measured selection lands (Faz 8):
    // first non-blacklisted registry entry.
    let model = models
        .models
        .iter()
        .find(|m| !m.blacklisted)
        .ok_or_else(|| {
            ribozym::Error::ConfigInvalid(
                "no non-blacklisted model in models.toml — seed the registry".into(),
            )
        })?
        .id
        .clone();
    let cloud = std::sync::Arc::new(ribozym::inference::CloudProvider::new(
        models.openrouter.base_url.clone(),
        api_key,
        model.clone(),
    ));
    let provider: std::sync::Arc<dyn ribozym::inference::InferenceProvider> =
        if models.local.eligible_task_types.is_empty() {
            cloud
        } else {
            let server = std::env::var(&models.local.server_url_env)
                .unwrap_or_else(|_| models.local.default_server_url.clone());
            let local = std::sync::Arc::new(ribozym::inference::LocalProvider::new(
                server,
                models.local.model_id.clone(),
            ));
            // VRAM probe stays at "always admits local" until the nvml
            // wiring lands (Faz 8); routing errors silently degrade to cloud.
            std::sync::Arc::new(ribozym::select::LocalFirstRouting::new(
                local,
                cloud,
                models.local.eligible_task_types.clone(),
                LOCAL_MODEL_LOAD_MIB,
                std::sync::Arc::new(|| u64::MAX),
            ))
        };
    // §8.2 call-boundary wall: every LLM turn passes through EnvLayer —
    // pre-clamp estimate against the REMAINING run budget (budget minus the
    // spend already in the ledger, so the wall survives process restarts),
    // E0 backoff for transient failures, actual-cost reconciliation. A
    // refusal surfaces as MODEL_BUDGET_DENIED and parks the run `Paused`
    // rather than failing the work.
    let spent_row = existing.as_ref().map(|(_, _, sp)| *sp).unwrap_or(0.0);
    let guarded: std::sync::Arc<dyn ribozym::inference::InferenceProvider> =
        std::sync::Arc::new(ribozym::inference::EnvLayer::new(
            std::sync::Arc::clone(&provider),
            ribozym::inference::price_of("openrouter"),
            (cfg.budget_usd - spent_row).max(0.0),
        ));
    let provider = guarded;
    let guard = std::sync::Arc::new(ribozym::repair::SchemaGuard::new(
        std::sync::Arc::clone(&provider),
        root.clone(),
    ));

    let resumed = existing.is_some();
    let mut o = match &existing {
        Some((status, _, _)) => {
            let planning_phases = [
                "idle",
                "spec_received",
                "clarifying",
                "researching",
                "advising",
                "spec_validated",
                "tasks_created",
            ];
            let plan = if planning_phases.contains(&status.as_str()) {
                Default::default()
            } else {
                let raw = std::fs::read_to_string(
                    cfg.root.join("runs").join(&cfg.spec_id).join("plan.json"),
                )
                .map_err(|e| {
                    ribozym::Error::ConfigInvalid(format!(
                        "resume needs runs/{}/plan.json: {e}",
                        cfg.spec_id
                    ))
                })?;
                serde_json::from_str(&raw)?
            };
            let mut o = Orchestrator::resume(&cfg, &plan)?;
            if o.unpause()? {
                eprintln!("resuming a paused run under the current budget");
            }
            o
        }
        None => Orchestrator::create_idle(&cfg)?,
    };

    // Management line (skipped by its own phase arms once past Scheduling).
    let mut planning = AgentPipeline::for_form(
        cfg.clone(),
        root.clone(),
        std::sync::Arc::clone(&guard),
        form_bytes.clone(),
    );
    match o.advance_planning(&mut planning)? {
        PlanningOutcome::Planned => {}
        PlanningOutcome::Waiting { gate } => {
            println!(
                "gate {} waits — vote with `ribozym approve {} {}`, then re-run",
                gate.as_str(),
                spec_id,
                gate.as_str()
            );
            return Ok(codes::APPROVAL);
        }
        PlanningOutcome::NeedsUser { why } => {
            println!("needs user: {why}");
            return Ok(codes::NEEDS_USER);
        }
        PlanningOutcome::Paused { reason } => {
            println!(
                "budget wall ({reason:?}) at a planning turn: run parked — raise budget, re-run"
            );
            return Ok(codes::BUDGET);
        }
    }
    let context = match std::fs::read_to_string(root.join("runs").join(&spec_id).join("spec.json"))
    {
        Ok(raw) => {
            let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
            format!("SPEC {} ({})", v["title"].as_str().unwrap_or(""), spec_id)
        }
        Err(_) => String::new(),
    };
    let gateway = std::sync::Arc::new(ribozym::gateway::ToolGateway::new(
        pc.toolchains.clone(),
        workdir.clone(),
        std::sync::Arc::new(ribozym::gateway::SystemRunner),
        std::env::vars().collect(),
    ));
    // The §13 web door: the configured backend (agent-reach via fetch_argv,
    // §13.1/Q5) with the run's OWN budget-domain clock and TTL from config.
    let fetcher: std::sync::Arc<dyn ribozym::web::WebFetcher> = match pc.web.fetcher.as_str() {
        "command" | "agent-reach" if !pc.web.fetch_argv.is_empty() => {
            std::sync::Arc::new(ribozym::web::CommandFetcher {
                argv: pc.web.fetch_argv.clone(),
                timeout_s: 60,
                output_cap_kb: 512,
            })
        }
        other => {
            // Configured but not wired to a command is an honest absence —
            // A11 tasks will fail loudly (WEB_UNAVAILABLE), nothing pretends.
            if other != "none" {
                eprintln!(
                    "warn: web fetcher {other:?} has no fetch_argv configured — A11 searches will fail"
                );
            }
            std::sync::Arc::new(ribozym::web::NullFetcher)
        }
    };
    let door = std::sync::Arc::new(ribozym::web::WebDoor {
        fetcher,
        db_path: root.join(".ribozym").join("state.db"),
        schema_dir: root.clone(),
        now_epoch: cfg.now_epoch,
        ttl_seconds: pc.web.cache_ttl_s,
    });
    let bridge = ribozym::agents::AgentBridge::new(
        root.clone(),
        std::sync::Arc::clone(&provider),
        gateway,
        &workdir,
        context,
    )
    .with_web(std::sync::Arc::clone(&door));
    let mut pipeline =
        AgentPipeline::for_resume(cfg.clone(), root.clone(), std::sync::Arc::clone(&guard));
    let out = o.advance_with(&bridge, &mut pipeline)?;
    let digest = o.final_digest()?;
    println!("spec_id:  {spec_id}");
    println!("run_id:   {}", cfg.run_id);
    println!("workdir:  {}", workdir.display());
    println!("phase:    {}", phase_of(&out));
    match &out {
        Outcome::Delivered => {
            let sp = planning.spec().ok();
            println!(
                "spent_usd: {:.4} / budget {:.4} ({} requirements)",
                o.spent_usd(),
                sp.as_ref()
                    .map(|s| s.budget.max_cost_usd)
                    .unwrap_or(cfg.budget_usd),
                sp.map(|s| s.requirements.len()).unwrap_or(0),
            );
        }
        Outcome::Waiting { gate } => println!(
            "gate {} waits — vote with `ribozym approve {} {}`, then re-run",
            gate.as_str(),
            spec_id,
            gate.as_str()
        ),
        Outcome::Paused { reason } => {
            println!("budget wall ({reason:?}): run parked — raise budget, re-run")
        }
        Outcome::NeedsUser { why } => println!("needs user: {why}"),
        Outcome::Failed { why } => println!("failed: {why}"),
    }
    if resumed {
        println!("(resumed)");
    }
    println!(
        "documents: runs/{spec_id}/raw_input.md, spec.json, plan.json, report.json, compliance.json"
    );
    println!("final hash: {digest}");
    Ok(outcome_code(&out))
}

/// Qwen2.5-Coder-7B Q4_K_M model load, measured (§10.1 admission math).
/// The nvml probe replacing the `u64::MAX` stub arrives with A10 (Faz 8).
const LOCAL_MODEL_LOAD_MIB: u64 = 4_600;

fn phase_of(out: &Outcome) -> &'static str {
    match out {
        Outcome::Delivered => "delivered",
        Outcome::Waiting { gate } => match gate {
            Gate::Spec => "awaiting spec gate",
            Gate::TaskPlan => "awaiting task_plan gate",
            Gate::Delivery => "awaiting delivery gate",
        },
        Outcome::NeedsUser { .. } => "needs_user",
        Outcome::Failed { .. } => "failed",
        Outcome::Paused { .. } => "paused_budget",
    }
}
