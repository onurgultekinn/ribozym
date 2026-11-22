//! Data models (§11). Each submodule maps 1:1 to a file in `schemas/`:
//!   spec.rs -> spec.schema.json        task.rs -> tasks.schema.json
//!   event.rs -> event.schema.json      execution.rs -> execution_result.schema.json
//!   report.rs -> department_report.schema.json
//!   compliance.rs -> compliance_check.schema.json
//!
//! Enum-valued fields are plain `String` here: the schema files are the single
//! source of validation truth (§11.0) — duplicating every enum in Rust invites
//! drift. Only `EventKind` is typed because the FSM dispatches on it (§6.1).
//!
//! Forward-compat (§11.0, no deny_unknown_fields): unknown fields are preserved
//! in `extra` and round-trip on serialization.

pub mod compliance;
pub mod event;
pub mod execution;
pub mod report;
pub mod spec;
pub mod task;

use serde_json::{Map, Value};

/// Preserved unknown fields (forward-compat, §11.0).
pub type Extra = Map<String, Value>;

pub trait ExtraFields {
    fn extra(&self) -> &Extra;
    fn extra_mut(&mut self) -> &mut Extra;
}

/// Flatten helper: derive both directions with one declaration per struct.
#[macro_export]
macro_rules! impl_extra {
    ($t:ty) => {
        impl $crate::models::ExtraFields for $t {
            fn extra(&self) -> &$crate::models::Extra {
                &self.extra
            }
            fn extra_mut(&mut self) -> &mut $crate::models::Extra {
                &mut self.extra
            }
        }
    };
}
