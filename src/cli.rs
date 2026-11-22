//! CLI — thin wrapper over the public API (P1, §24 rule 5; §20.1).
//! Exit codes follow §20.2: 0 success, 1 user error, 2 NeedsUser,
//! 3 BUDGET_EXCEEDED, 4 approval timeout/denied, 5 unexpected crash.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "ribozym",
    version,
    about = "Autonomous multi-agent R&D ecosystem"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Config-file operations (§20.1).
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Document validation against schemas/ contracts (§20.1).
    Schema {
        #[command(subcommand)]
        command: SchemaCommand,
    },
    /// Drive a run. `--mode interactive` runs the full A1→A3 management line
    /// from a filled spec form (§6.1, §5.4: draft → spec gate → plan →
    /// execute → report → referee); `--mode mini` runs the deterministic
    /// Faz-4 golden path (FSM+DAG+ledger+gates) over a tasks.json with a
    /// scripted executor (§22).
    Run {
        /// "interactive" (real agents) | "mini" (deterministic, no LLM).
        #[arg(long, default_value = "mini")]
        mode: String,
        /// Filled spec-form markdown (required for `--mode interactive`).
        #[arg(long)]
        form: Option<String>,
        /// Spec id; default derives from the form date + hash.
        #[arg(long)]
        spec_id: Option<String>,
        /// Isolated working copy agents write into (§12.6).
        #[arg(long)]
        workdir: Option<String>,
        /// tasks.json document for `--mode mini`.
        #[arg(long)]
        tasks: Option<String>,
        /// spec.json to enforce §6.4 traceability against (the plan must cover
        /// every requirement). Without it the cross-doc gate cannot run and is
        /// reported as skipped — never silently assumed.
        #[arg(long)]
        spec: Option<String>,
        /// Batch approval: all gates auto-approved (§9), stop-conditions stay.
        #[arg(long)]
        yes: bool,
        /// Autonomy level; may only LOWER the spec's own level (§9).
        #[arg(long)]
        autonomy: Option<u8>,
        /// Assume FinalReview compliance (the mini path has no A1 verdict yet).
        /// Off ⇒ FinalReview takes the ComplianceFailed→Crisis route, so the
        /// delivery claim is never silently hardcoded true.
        #[arg(long)]
        assume_compliant: bool,
    },
    /// Run status — derived truth from state.db (phase, tasks, spend, open
    /// gate). No spec id: every run in this project, one row each.
    Status { spec_id: Option<String> },
    /// Print the sealed A3 report (§5.7 m4) — the artifact, never a rewrite.
    Report {
        spec_id: String,
        /// "md" (human) | "json" (the sealed document verbatim).
        #[arg(long, default_value = "md")]
        format: String,
    },
    /// Budget frame from config + per-run spend from the ledger.
    Budget {
        #[command(subcommand)]
        command: BudgetCommand,
    },
    /// Write a blank spec-form template (§5.4) — refuses to clobber.
    NewForm {
        /// Output path (default: ./spec_form.md).
        #[arg(long, default_value = "spec_form.md")]
        out: String,
    },
    /// File-based gate vote for a waiting run (§9 — the remote/CI path).
    Approve {
        spec_id: String,
        /// spec | task_plan | delivery
        gate: String,
        #[arg(long)]
        note: Option<String>,
        /// Record a denial instead of approval.
        #[arg(long)]
        deny: bool,
        #[arg(long, default_value = "cli-user")]
        decided_by: String,
    },
}

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Validate config files against the registry invariants.
    Validate,
}

#[derive(Subcommand)]
pub enum BudgetCommand {
    /// Show the budget frame and per-run spend.
    Show,
}

#[derive(Subcommand)]
pub enum SchemaCommand {
    /// Validate a JSON document against one of the schemas/ contracts.
    Validate {
        /// e.g. spec.schema.json
        schema: String,
        /// Path to the JSON document.
        #[arg(long)]
        file: String,
        /// Full draft-2020-12 validation via the bundled schemas (default:
        /// structural subset).
        #[arg(long)]
        full: bool,
    },
}
