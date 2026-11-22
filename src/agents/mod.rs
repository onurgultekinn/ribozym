//! Concrete agents. Two families:
//! * DAG-dispatchable workers (§6.3 pipeline): A5 coder, A7 security, A4
//!   test architect — reachable through [`AgentBridge`] + [`registry`].
//! * Control-plane hats (Faz 5-c): A1 architect/referee, A2 CEO planner,
//!   A3 chief reporter — the driver calls their explicit methods between
//!   phases (§6.1), so they are NOT DAG tasks and NOT in [`registry`]; a
//!   plan that assigns work to them is refused at planning time (a2).
//!
//! [`AgentBridge`] is the Faz-5 seam decision (PROGRESS "Sıradaki"): the
//! control plane stays SYNCHRONOUS and deterministic (§7.5 single writer,
//! golden-digest stability) while agents are `async_trait` (§5.3). The bridge
//! runs each task on a dedicated current-thread tokio runtime via
//! `block_on` — one runtime per task, no shared reactor, no cross-task
//! state. Costs: a thread + runtime per task execution, trivially small at
//! DAG concurrency ≤ `max_concurrency`.

pub mod a1;
pub mod a11;
pub mod a2;
pub mod a3;
pub mod a4;
pub mod a5;
pub mod a7;
pub mod bridge;

pub use a1::A1Architect;
pub use a11::A11Web;
pub use a2::A2Ceo;
pub use a3::{A3Reporter, RunFacts, TaskFact};
pub use a4::A4TestArchitect;
pub use a5::A5Coder;
pub use a7::A7Security;
pub use bridge::AgentBridge;

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::agent::Agent;
use crate::error::{Error, Result};

/// Registry: id → agent. DAG-dispatchable workers only; the control-plane
/// hats (A1/A2/A3) are phase-callable, not task-executable. An id outside
/// this map returns a clear error instead of silently succeeding — an
/// unmapped task must never be "executed" by nothing.
pub fn registry() -> BTreeMap<&'static str, Arc<dyn Agent>> {
    let mut m: BTreeMap<&'static str, Arc<dyn Agent>> = BTreeMap::new();
    for a in [
        Arc::new(A5Coder::new()) as Arc<dyn Agent>,
        Arc::new(A7Security::new()) as Arc<dyn Agent>,
        Arc::new(A4TestArchitect::new()) as Arc<dyn Agent>,
        Arc::new(A11Web::new()) as Arc<dyn Agent>,
    ] {
        m.insert(a.id(), a);
    }
    m
}

pub fn lookup(id: &str) -> Result<Arc<dyn Agent>> {
    registry().get(id).cloned().ok_or_else(|| {
        Error::Agent(format!(
            "agent {id:?} not in the MVP registry (Faz 5 scope)"
        ))
    })
}
