//! Event envelope — `schemas/event.schema.json` (§7.3).
//!
//! `EventKind` is the one typed enum: the FSM dispatches on it (§6.1).
//! Unknown event names deserialize into `EventKind::Unknown` so forward-compat
//! holds and resume never drops an unrecognized record.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Extra;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventEnvelope {
    pub seq: u64,
    pub ts: String,
    pub spec_id: String,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub event: EventKind,
    pub correlation_id: String,
    pub schema_version: String,
    #[serde(default)]
    pub data: Extra,
}

/// Catalog in `schemas/event.schema.json` + Ek A. New names land in `Unknown`
/// until the schema and this enum are updated together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    // Lifecycle
    SpecReceived,
    SpecValidated,
    SpecRejected,
    SpecRevised,
    // Planning
    TasksCreated,
    TasksRevised,
    TaskGraphRejected,
    // Execution
    TaskStarted,
    TaskCompleted,
    TaskFailed,
    TaskRetried,
    TaskSkipped,
    TaskBlocked,
    TaskOwnershipConflict,
    // Escalation
    EscalatedToCEO,
    CrisisResolved,
    EscalatedToCloud,
    EscalatedToUser,
    // Human gates
    GateRequested,
    GateTimeout,
    HumanApprovalRequested,
    HumanApprovalGranted,
    HumanApprovalDenied,
    // User ideas
    UserIdeaProposed,
    UserIdeaIntegrated,
    UserIdeaRejected,
    // Delivery
    Delivered,
    DeliveryRejected,
    // Budget
    BudgetWarning,
    BudgetExceeded,
    MonthlyBudgetExceeded,
    // Model
    ModelSelected,
    ModelBlacklisted,
    NewModelDiscovered,
    ModelRepairTried,
    // Web
    WebSearchRequested,
    WebSearchCompleted,
    VerificationScored,
    WebContentFlaggedInjection,
    // Archive
    ArchiveCorrupt,
    // System
    PreflightFailed,
    SandboxViolation,
    LockStolen,
    #[serde(other)]
    Unknown,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SpecReceived => "SpecReceived",
            Self::SpecValidated => "SpecValidated",
            Self::SpecRejected => "SpecRejected",
            Self::SpecRevised => "SpecRevised",
            Self::TasksCreated => "TasksCreated",
            Self::TasksRevised => "TasksRevised",
            Self::TaskGraphRejected => "TaskGraphRejected",
            Self::TaskStarted => "TaskStarted",
            Self::TaskCompleted => "TaskCompleted",
            Self::TaskFailed => "TaskFailed",
            Self::TaskRetried => "TaskRetried",
            Self::TaskSkipped => "TaskSkipped",
            Self::TaskBlocked => "TaskBlocked",
            Self::TaskOwnershipConflict => "TaskOwnershipConflict",
            Self::EscalatedToCEO => "EscalatedToCEO",
            Self::CrisisResolved => "CrisisResolved",
            Self::EscalatedToCloud => "EscalatedToCloud",
            Self::EscalatedToUser => "EscalatedToUser",
            Self::GateRequested => "GateRequested",
            Self::GateTimeout => "GateTimeout",
            Self::HumanApprovalRequested => "HumanApprovalRequested",
            Self::HumanApprovalGranted => "HumanApprovalGranted",
            Self::HumanApprovalDenied => "HumanApprovalDenied",
            Self::UserIdeaProposed => "UserIdeaProposed",
            Self::UserIdeaIntegrated => "UserIdeaIntegrated",
            Self::UserIdeaRejected => "UserIdeaRejected",
            Self::Delivered => "Delivered",
            Self::DeliveryRejected => "DeliveryRejected",
            Self::BudgetWarning => "BudgetWarning",
            Self::BudgetExceeded => "BudgetExceeded",
            Self::MonthlyBudgetExceeded => "MonthlyBudgetExceeded",
            Self::ModelSelected => "ModelSelected",
            Self::ModelBlacklisted => "ModelBlacklisted",
            Self::NewModelDiscovered => "NewModelDiscovered",
            Self::ModelRepairTried => "ModelRepairTried",
            Self::WebSearchRequested => "WebSearchRequested",
            Self::WebSearchCompleted => "WebSearchCompleted",
            Self::VerificationScored => "VerificationScored",
            Self::WebContentFlaggedInjection => "WebContentFlaggedInjection",
            Self::ArchiveCorrupt => "ArchiveCorrupt",
            Self::PreflightFailed => "PreflightFailed",
            Self::SandboxViolation => "SandboxViolation",
            Self::LockStolen => "LockStolen",
            Self::Unknown => "Unknown",
        }
    }
}

/// Convenience for building `data` payloads.
pub fn data_from_pairs<const N: usize>(pairs: [(&str, Value); N]) -> Extra {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}
