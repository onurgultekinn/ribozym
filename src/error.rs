//! Error taxonomy (§16). Prefixes mirror the error table; this enum covers the
//! control-plane errors that exist after Phase 2. New variants join the table.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("TOOLCHAIN_CHECK_FAILED: {0}")]
    ToolchainCheck(String),

    #[error("{0}")]
    ToolchainNotAllowed(String),

    #[error("CONFIG_INVALID: {0}")]
    ConfigInvalid(String),

    #[error("MODEL_SCHEMA_INVALID: {0}")]
    ModelSchemaInvalid(String),

    /// §16 MODEL/BUDGET: pre-clamp refusal — the call is never made (§8.2 m3).
    #[error("MODEL_BUDGET_DENIED: {0}")]
    ModelBudgetDenied(String),

    /// §6.1/§6.2: the driver reached a phase/trigger pair the tables forbid —
    /// always a control-plane wiring bug, surfaced never swallowed.
    #[error("FSM: {0}")]
    Fsm(String),

    #[error("SCHEDULER: {0}")]
    Scheduler(String),

    #[error("LEDGER: {0}")]
    Ledger(String),

    /// §16 AGENT-family: the WORK failed though the protocol held — fix-loop
    /// exhaustion (§8.1 E1), semantically wrong-but-schema-valid model output,
    /// or a dispatch to an agent outside the registry. Distinct from
    /// ModelSchemaInvalid (repaired in place): Agent means the ladder (§8)
    /// owns the next attempt.
    #[error("AGENT: {0}")]
    Agent(String),

    /// §13: the web door — no backend, invalid query, refused content. The
    /// search FAILS LOUD; it never quietly returns fewer sources than asked.
    #[error("WEB_UNAVAILABLE: {0}")]
    WebUnavailable(String),

    /// §7.6/B2: an archive section failed its manifest hash at read time.
    /// Silent continue is forbidden — the caller either repairs from the
    /// live source document or parks the run for the human.
    #[error("ARCHIVE_CORRUPT: {0}")]
    ArchiveCorrupt(String),

    /// §6.4: the task graph cannot reach `TasksCreated` — a requirement is
    /// uncovered or a task cites a phantom id. A user-correctable spec/plan
    /// error (§20.2 exit 1), raised at the gate before any run row is written.
    #[error("TRACEABILITY: {0}")]
    Traceability(String),

    /// §21.2 discrete-crash variant (v2.47 B4): the injected kill. Only the
    /// test harness arms it; a production driver never sees this variant.
    #[error("CRASH_SIMULATED after {0:?}")]
    Crashed(crate::orchestrator::Stage),

    #[error(transparent)]
    Provider(#[from] Box<crate::inference::ProviderError>),

    #[error("ENV_LAYER_EXHAUSTED: {0}")]
    EnvLayerExhausted(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Toml(#[from] toml::de::Error),

    #[error(transparent)]
    Db(#[from] rusqlite::Error),
}
