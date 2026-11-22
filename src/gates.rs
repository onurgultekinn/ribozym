//! Approval gates (§9). Three gates, one policy table, one audit trail.
//!
//! | Gate        | Level 1 | Level 2 | Level 3        |
//! | ----------- | ------- | ------- | -------------- |
//! | spec        | required| required| auto           |
//! | task_plan   | required| auto    | auto           |
//! | delivery    | required| required| optional(--yes)|
//!
//! Q4 risk rule: a gate NEVER stops production — while it waits, isolated
//! work continues and only the terminal step is locked. Low-risk gates
//! (`spec`, `task_plan`) auto-approve after `gate_timeout_seconds` with a
//! `timeout_auto` audit row; `delivery` has no auto-approval — the run parks
//! as NeedsUser and nothing is lost. `--yes` batch approves every gate but
//! cannot overrule an explicit denial already on file.

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Spec,
    TaskPlan,
    Delivery,
}

impl Gate {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spec => "spec",
            Self::TaskPlan => "task_plan",
            Self::Delivery => "delivery",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "spec" => Self::Spec,
            "task_plan" => Self::TaskPlan,
            "delivery" => Self::Delivery,
            _ => return None,
        })
    }
    /// Q4: only delivery is high-risk (touches the mainline; user decision).
    pub fn is_low_risk(self) -> bool {
        !matches!(self, Self::Delivery)
    }
}

/// How a gate behaves for an autonomy level (§9 table). `Auto` = passes
/// without waiting for a human at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Required,
    Auto,
}

pub fn mode_for(level: u8, gate: Gate, batch_yes: bool) -> Mode {
    // "Toplu (--yes): tüm kapılar otomatik onaylıdır" — denial handling is
    // the caller's stop-condition check, done BEFORE consulting this.
    if batch_yes {
        return Mode::Auto;
    }
    // §9 table, spelled out per level — no clever fall-through, easy to audit.
    match level {
        1 => Mode::Required,
        2 => match gate {
            Gate::TaskPlan => Mode::Auto,
            _ => Mode::Required,
        },
        _ => match gate {
            // level 3: only delivery still needs a human (--yes covers it above)
            Gate::Delivery => Mode::Required,
            _ => Mode::Auto,
        },
    }
}

/// `approvals.decision` vocabulary. The migration comment lists the decision
/// values; `pending` is the durable GATE REQUEST record — §9's timeout clock
/// must survive `run` → `resume` (fresh process), so the request instant
/// lives in the DB, never in memory.
pub mod decision {
    pub const APPROVED: &str = "approved";
    pub const DENIED: &str = "denied";
    pub const TIMEOUT_AUTO: &str = "timeout_auto";
    pub const PENDING: &str = "pending";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRow {
    pub decided_by: String,
    pub decision: String, // approved | denied | timeout_auto | pending
    pub note: Option<String>,
    /// The `ts` column: for `pending` rows this is the §9 clock anchor.
    pub ts: String,
}

/// Why a gate opened — recorded in the audit note for timeout approvals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantedReason {
    Human,
    BatchYes,
    LevelAuto,
    /// §9 Q4: waited out the low-risk timeout.
    TimeoutAuto,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    Granted(GrantedReason),
    Denied {
        by: String,
    },
    /// High-risk gate, no decision, timeout spent → park as NeedsUser.
    Park,
    /// Still inside the timeout window; the orchestrator keeps scheduling.
    StillWaiting,
}

/// Inputs to one `decide` call. Pure §9 policy — the orchestrator polls with
/// its own (injectable) clock, so the timeout is testable without sleeping.
pub struct GateRequest<'a> {
    pub gate: Gate,
    pub autonomy_level: u8,
    pub batch_yes: bool,
    pub requested_at_epoch: u64,
    pub now_epoch: u64,
    pub timeout_seconds: u64,
    pub existing: Option<&'a ApprovalRow>,
}

/// Recover the ORIGINAL grant reason from an audit row's `decided_by` field.
/// The reason must be read from what was recorded, never re-derived from the
/// current `--yes`/autonomy context — otherwise a `--yes` auto-approval would
/// silently relabel as "human" on a later re-entry (§9: denetim izi eksiksiz).
fn approved_reason(decided_by: &str) -> GrantedReason {
    match decided_by {
        "batch-yes" => GrantedReason::BatchYes,
        "autonomy-policy" => GrantedReason::LevelAuto,
        "gate-timeout" => GrantedReason::TimeoutAuto,
        _ => GrantedReason::Human,
    }
}

pub fn decide(req: GateRequest<'_>) -> GateOutcome {
    // An explicit human decision row is the audit truth and wins over any
    // policy — EXCEPT `pending`, which is the durable REQUEST record: the §9
    // timeout clock runs from its `ts` (supplied as `requested_at_epoch`).
    if let Some(row) = req.existing {
        match row.decision.as_str() {
            decision::APPROVED => return GateOutcome::Granted(approved_reason(&row.decided_by)),
            decision::DENIED => {
                return GateOutcome::Denied {
                    by: row.decided_by.clone(),
                }
            }
            decision::TIMEOUT_AUTO => return GateOutcome::Granted(GrantedReason::TimeoutAuto),
            decision::PENDING => {}
            other => {
                return GateOutcome::Denied {
                    by: format!("corrupt-approval-decision({other})"),
                }
            }
        }
    }
    match mode_for(req.autonomy_level, req.gate, req.batch_yes) {
        Mode::Auto => GateOutcome::Granted(if req.batch_yes {
            GrantedReason::BatchYes
        } else {
            GrantedReason::LevelAuto
        }),
        Mode::Required => {
            let waited = req.now_epoch.saturating_sub(req.requested_at_epoch);
            if waited >= req.timeout_seconds {
                if req.gate.is_low_risk() {
                    GateOutcome::Granted(GrantedReason::TimeoutAuto)
                } else {
                    GateOutcome::Park
                }
            } else {
                GateOutcome::StillWaiting
            }
        }
    }
}

pub fn reason_str(r: GrantedReason) -> &'static str {
    match r {
        GrantedReason::Human => "human",
        GrantedReason::BatchYes => "batch_yes",
        GrantedReason::LevelAuto => "level_auto",
        GrantedReason::TimeoutAuto => "timeout_auto",
    }
}

/// Ledger of gate decisions (§7.2 `approvals`). PRIMARY KEY (spec_id, gate):
/// one decision per gate per spec — a re-vote REPLACES and the old audit row
/// is preserved in events.jsonl (HumanApproval* events), never silently lost.
pub struct Approvals<'a> {
    conn: &'a Connection,
}

impl<'a> Approvals<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn record(
        &self,
        spec_id: &str,
        gate: Gate,
        decided_by: &str,
        decision: &str,
        note: Option<&str>,
        ts: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO approvals(spec_id, gate, decided_by, decision, note, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![spec_id, gate.as_str(), decided_by, decision, note, ts],
        )?;
        Ok(())
    }

    pub fn lookup(&self, spec_id: &str, gate: Gate) -> Result<Option<ApprovalRow>> {
        let row = self.conn
            .query_row(
                "SELECT decided_by, decision, note, ts FROM approvals WHERE spec_id = ?1 AND gate = ?2",
                params![spec_id, gate.as_str()],
                |r| {
                    Ok(ApprovalRow {
                        decided_by: r.get(0)?,
                        decision: r.get(1)?,
                        note: r.get(2)?,
                        ts: r.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::StateStore;

    fn with_approvals(f: impl FnOnce(&Approvals, &Connection)) {
        let store = StateStore::open_in_memory().unwrap();
        f(&Approvals::new(store.conn()), store.conn());
    }

    #[test]
    fn autonomy_matrix_is_the_plan_table() {
        // §9, row by row.
        for gate in [Gate::Spec, Gate::TaskPlan, Gate::Delivery] {
            assert_eq!(mode_for(1, gate, false), Mode::Required, "L1 everything");
        }
        assert_eq!(mode_for(2, Gate::Spec, false), Mode::Required);
        assert_eq!(
            mode_for(2, Gate::TaskPlan, false),
            Mode::Auto,
            "L2 task_plan auto"
        );
        assert_eq!(mode_for(2, Gate::Delivery, false), Mode::Required);
        assert_eq!(mode_for(3, Gate::Spec, false), Mode::Auto);
        assert_eq!(mode_for(3, Gate::TaskPlan, false), Mode::Auto);
        assert_eq!(
            mode_for(3, Gate::Delivery, false),
            Mode::Required,
            "L3 delivery optional = only via --yes"
        );
        assert_eq!(
            mode_for(3, Gate::Delivery, true),
            Mode::Auto,
            "--yes unlocks it"
        );
        assert_eq!(
            mode_for(9, Gate::Delivery, false),
            Mode::Required,
            "unknown level → safest"
        );
    }

    fn req(
        gate: Gate,
        level: u8,
        yes: bool,
        waited: u64,
        row: Option<&ApprovalRow>,
    ) -> GateRequest<'_> {
        GateRequest {
            gate,
            autonomy_level: level,
            batch_yes: yes,
            requested_at_epoch: 1000,
            now_epoch: 1000 + waited,
            timeout_seconds: 1200,
            existing: row,
        }
    }

    #[test]
    fn human_decision_on_file_wins_over_everything() {
        let approved = ApprovalRow {
            decided_by: "user".into(),
            decision: "approved".into(),
            note: None,
            ts: "t".into(),
        };
        let denied = ApprovalRow {
            decided_by: "user".into(),
            decision: "denied".into(),
            note: Some("scope".into()),
            ts: "t".into(),
        };
        // Even --yes cannot overrule an explicit denial (stop conditions stay, §9).
        assert_eq!(
            decide(req(Gate::Delivery, 3, true, 0, Some(&denied))),
            GateOutcome::Denied { by: "user".into() }
        );
        assert_eq!(
            decide(req(Gate::Spec, 1, false, 0, Some(&approved))),
            GateOutcome::Granted(GrantedReason::Human)
        );
    }

    #[test]
    fn pending_row_is_a_clock_anchor_not_a_decision() {
        let pending = ApprovalRow {
            decided_by: "gate-request".into(),
            decision: "pending".into(),
            note: None,
            ts: "t".into(),
        };
        // inside the window → still waiting (mode Required); batch/--yes passes.
        assert_eq!(
            decide(req(Gate::TaskPlan, 1, false, 600, Some(&pending))),
            GateOutcome::StillWaiting
        );
        // window spent → low-risk auto, high-risk park.
        assert_eq!(
            decide(req(Gate::TaskPlan, 1, false, 1200, Some(&pending))),
            GateOutcome::Granted(GrantedReason::TimeoutAuto)
        );
        assert_eq!(
            decide(req(Gate::Delivery, 1, false, 1200, Some(&pending))),
            GateOutcome::Park
        );
        assert_eq!(
            decide(req(Gate::TaskPlan, 1, true, 0, Some(&pending))),
            GateOutcome::Granted(GrantedReason::BatchYes)
        );
    }

    #[test]
    fn low_risk_timeout_auto_approves_delivery_does_not() {
        assert_eq!(
            decide(req(Gate::TaskPlan, 1, false, 1200, None)),
            GateOutcome::Granted(GrantedReason::TimeoutAuto)
        );
        assert_eq!(
            decide(req(Gate::Delivery, 1, false, 1200, None)),
            GateOutcome::Park,
            "high-risk: no auto approval, park"
        );
        assert_eq!(
            decide(req(Gate::Spec, 1, false, 1199, None)),
            GateOutcome::StillWaiting,
            "one second short of timeout"
        );
    }

    #[test]
    fn timeout_auto_row_regrants_as_timeout_auto() {
        // A previously timeout-approved gate re-encountered on resume must
        // pass identically (idempotent audit), not fall to corrupt handling.
        let row = ApprovalRow {
            decided_by: "gate-timeout".into(),
            decision: "timeout_auto".into(),
            note: Some("timeout_auto".into()),
            ts: "t".into(),
        };
        assert_eq!(
            decide(req(Gate::Spec, 1, false, 0, Some(&row))),
            GateOutcome::Granted(GrantedReason::TimeoutAuto)
        );
    }

    #[test]
    fn grant_reason_is_read_from_audit_not_re_derived_from_mode() {
        // A --yes auto-approval re-encountered later (context changed) must
        // still report BatchYes — never relabel as Human. This is the audit
        // trail §9 demands ("kim, neyi, ne zaman, hangi notla").
        let batch = ApprovalRow {
            decided_by: "batch-yes".into(),
            decision: "approved".into(),
            note: Some("batch_yes".into()),
            ts: "t".into(),
        };
        assert_eq!(
            decide(req(Gate::TaskPlan, 1, false, 0, Some(&batch))),
            GateOutcome::Granted(GrantedReason::BatchYes),
            "batch provenance survives a non-batch re-entry"
        );
        let policy = ApprovalRow {
            decided_by: "autonomy-policy".into(),
            decision: "approved".into(),
            note: Some("level_auto".into()),
            ts: "t".into(),
        };
        assert_eq!(
            decide(req(Gate::Spec, 3, false, 0, Some(&policy))),
            GateOutcome::Granted(GrantedReason::LevelAuto)
        );
    }

    #[test]
    fn auto_modes_never_wait() {
        assert_eq!(
            decide(req(Gate::TaskPlan, 2, false, 0, None)),
            GateOutcome::Granted(GrantedReason::LevelAuto)
        );
        assert_eq!(
            decide(req(Gate::Spec, 1, true, 0, None)),
            GateOutcome::Granted(GrantedReason::BatchYes)
        );
    }

    #[test]
    fn corrupt_decision_row_is_denied_not_crashed() {
        let row = ApprovalRow {
            decided_by: "x".into(),
            decision: "maybe".into(),
            note: None,
            ts: "t".into(),
        };
        let out = decide(req(Gate::Spec, 1, false, 0, Some(&row)));
        assert!(matches!(out, GateOutcome::Denied { .. }));
    }

    #[test]
    fn record_and_lookup_round_trip_with_note() {
        with_approvals(|a, _conn| {
            assert_eq!(a.lookup("spec-1", Gate::Delivery).unwrap(), None);
            a.record("spec-1", Gate::Spec, "user", "approved", Some("lgtm"), "t1")
                .unwrap();
            let row = a.lookup("spec-1", Gate::Spec).unwrap().unwrap();
            assert_eq!(row.decided_by, "user");
            assert_eq!(row.note.as_deref(), Some("lgtm"));
            // Re-vote replaces (audit trail lives in events.jsonl).
            a.record("spec-1", Gate::Spec, "user2", "denied", None, "t2")
                .unwrap();
            let row = a.lookup("spec-1", Gate::Spec).unwrap().unwrap();
            assert_eq!(row.decision, "denied");
            assert_eq!(row.decided_by, "user2");
            // gates are keyed independently
            assert!(a.lookup("spec-1", Gate::TaskPlan).unwrap().is_none());
        });
    }

    #[test]
    fn timeout_auto_reason_string_matches_plan_vocabulary() {
        // §9 names the event reason exactly "timeout_auto".
        assert_eq!(reason_str(GrantedReason::TimeoutAuto), "timeout_auto");
    }
}
