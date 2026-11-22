//! Operation ledger (§7.4). The `ledger` table is the proof of what actually
//! happened; resume reads it to close the atomicity window that the §7.4 v2.47
//! B4 write order deliberately shrinks.
//!
//! Fixed write order for every operation (B4):
//!
//! ```text
//! (1) INSERT state='started'   ← this module
//! (2) state.db transaction     ← caller (task/run rows commit together)
//! (3) events.jsonl append      ← caller (EventLog, fsynced per line)
//! (4) UPDATE state='finished'  ← this module
//! ```
//!
//! op_id is DETERMINISTIC: `op-<short(sha256(kind|spec|task|payload_hash))>`.
//! A crashed run re-deriving the same operation gets the same op_id, so
//! "already finished → skip" (§7.4 rule 4) is a primary-key check, not a scan.

use rusqlite::{params, Connection};

use crate::error::Result;
use crate::hash;

/// §7.2 kind column.
pub mod kind {
    pub const AGENT_CALL: &str = "agent_call";
    pub const SHELL_EXEC: &str = "shell_exec";
    pub const GIT_OP: &str = "git_op";
    pub const WEB_FETCH: &str = "web_fetch";
    /// Faz 5-d: PLANNING turns (A1 draft, A2 plan) — model calls that belong
    /// to no DAG task. Same §7.4 write order, same exactly-once charge, no
    /// task-row/event coupling.
    pub const PLAN_TURN: &str = "plan_turn";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Op {
    pub op_id: String,
    pub kind: String,
    pub spec_id: String,
    pub task_id: Option<String>,
    pub payload_hash: String,
    /// `started | finished | failed`
    pub state: String,
    pub result_ref: Option<String>,
}

/// What §7.4 step 2 says to do with an op left at `started`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeAction {
    /// `agent_call` with `result_ref` set: the work is DONE, only the
    /// persistence was interrupted — rewrite the result, never re-call.
    FinalizeResult,
    /// Re-runnable: shells/agent calls re-execute with the same input hash
    /// (a fresh attempt on identical input is explicitly allowed by §7.4).
    ReRun,
    /// `state='finished'` — the proof that a completed task never re-runs.
    Done,
}

pub fn plan_resume(op: &Op) -> ResumeAction {
    match op.state.as_str() {
        "finished" | "failed" => ResumeAction::Done,
        _ => {
            // A planning turn with a persisted result is the same "iş
            // bitmiştir, yalnız yazma kesilmiştir" case as an agent_call.
            if matches!(op.kind.as_str(), kind::AGENT_CALL | kind::PLAN_TURN)
                && op.result_ref.is_some()
            {
                ResumeAction::FinalizeResult
            } else {
                // shell_exec/git_op/web_fetch are re-runnable tools (§7.4:
                // "cargo/git/docker tümü böyle"); git SHA-skip needs the real
                // remote and lands with A9 in Faz 6 — re-run is the safe,
                // idempotent-by-input default here.
                ResumeAction::ReRun
            }
        }
    }
}

pub struct Ledger<'a> {
    conn: &'a Connection,
}

impl<'a> Ledger<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Deterministic operation identity from the payload.
    pub fn op_id(
        kind: &str,
        spec_id: &str,
        task_id: Option<&str>,
        payload: &serde_json::Value,
    ) -> String {
        let id = hash::hash_json(&serde_json::json!([
            kind,
            spec_id,
            task_id.unwrap_or(""),
            payload
        ]));
        format!("op-{}", &id[..20])
    }

    /// Step (1). `INSERT OR IGNORE`: a crash between (1) and (4) re-derives the
    /// same op_id, so restarting the operation is idempotent and does not
    /// fabricate a second ledger row. Returns false if the op already existed.
    pub fn start(
        &self,
        op_kind: &str,
        spec_id: &str,
        task_id: Option<&str>,
        payload: &serde_json::Value,
        started_at: &str,
    ) -> Result<String> {
        let op_id = Self::op_id(op_kind, spec_id, task_id, payload);
        let payload_hash = hash::hash_json(payload);
        self.conn.execute(
            "INSERT OR IGNORE INTO ledger(op_id, kind, spec_id, task_id, payload_hash, state, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'started', ?6)",
            params![op_id, op_kind, spec_id, task_id, payload_hash, started_at],
        )?;
        Ok(op_id)
    }

    /// Persist the result pointer while work is finishing — this is what makes
    /// the §7.4 `agent_call` "iş bitmiştir, yalnız yazma kesilmiştir" branch
    /// detectable on resume.
    pub fn set_result(&self, op_id: &str, result_ref: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE ledger SET result_ref = ?2 WHERE op_id = ?1",
            params![op_id, result_ref],
        )?;
        Ok(())
    }

    /// Step (4).
    pub fn finish(&self, op_id: &str, result_ref: Option<&str>, finished_at: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE ledger SET state = 'finished', result_ref = COALESCE(?2, result_ref), finished_at = ?3
             WHERE op_id = ?1",
            params![op_id, result_ref, finished_at],
        )?;
        Ok(())
    }

    /// Terminal failure marker — resume treats `failed` as closed (the retry
    /// ladder in §8 owns the NEXT attempt, it does not resurrect this row).
    pub fn fail(&self, op_id: &str, finished_at: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE ledger SET state = 'failed', finished_at = ?2 WHERE op_id = ?1",
            params![op_id, finished_at],
        )?;
        Ok(())
    }

    /// §8.2 charge guard (migration 0003): flips `charged_at` NULL→stamp
    /// atomically; `true` means THIS caller won the right to book the op's
    /// cost into `runs.spent_usd`. Must run INSIDE the same BEGIN IMMEDIATE
    /// as the spend increment, which must be RELATIVE (the row's own value
    /// plus the cost). Otherwise the §7.4-B4 kill window between the commit
    /// and the ledger close double-charges on resume: the FinalizeResult
    /// branch re-books the persisted cost while the row already carries it.
    pub fn charge_once(&self, op_id: &str, charged_at: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE ledger SET charged_at = ?2 WHERE op_id = ?1 AND charged_at IS NULL",
            params![op_id, charged_at],
        )?;
        Ok(n == 1)
    }

    pub fn get(&self, op_id: &str) -> Result<Option<Op>> {
        let mut stmt = self.conn.prepare(
            "SELECT op_id, kind, spec_id, task_id, payload_hash, state, result_ref
             FROM ledger WHERE op_id = ?1",
        )?;
        let row = stmt.query_row(params![op_id], |r| {
            Ok(Op {
                op_id: r.get(0)?,
                kind: r.get(1)?,
                spec_id: r.get(2)?,
                task_id: r.get(3)?,
                payload_hash: r.get(4)?,
                state: r.get(5)?,
                result_ref: r.get(6)?,
            })
        });
        match row {
            Ok(op) => Ok(Some(op)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// §7.4 step 1: everything still open for this run's spec. Deterministic
    /// order (op_id) so replay is reproducible.
    pub fn unfinished(&self, spec_id: &str) -> Result<Vec<Op>> {
        let mut stmt = self.conn.prepare(
            "SELECT op_id, kind, spec_id, task_id, payload_hash, state, result_ref
             FROM ledger WHERE spec_id = ?1 AND state = 'started' ORDER BY op_id",
        )?;
        let rows = stmt.query_map(params![spec_id], |r| {
            Ok(Op {
                op_id: r.get(0)?,
                kind: r.get(1)?,
                spec_id: r.get(2)?,
                task_id: r.get(3)?,
                payload_hash: r.get(4)?,
                state: r.get(5)?,
                result_ref: r.get(6)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// §7.4 rule 4: the PROOF a task operation already completed.
    pub fn is_finished(&self, op_id: &str) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM ledger WHERE op_id = ?1 AND state = 'finished'",
            params![op_id],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::StateStore;
    use serde_json::json;

    /// Open a store, hand its connection to the closure with a Ledger on top —
    /// no leaked handles, the borrow checker does the lifetime work.
    fn with_ledger(f: impl FnOnce(&Ledger)) {
        let store = StateStore::open_in_memory().unwrap();
        f(&Ledger::new(store.conn()));
    }

    fn payload() -> serde_json::Value {
        json!({"task": "T1", "prompt": "x", "i": 1})
    }

    #[test]
    fn op_id_is_deterministic_from_payload() {
        let a = Ledger::op_id(kind::AGENT_CALL, "spec-1", Some("T1"), &payload());
        let b = Ledger::op_id(
            kind::AGENT_CALL,
            "spec-1",
            Some("T1"),
            &json!({"i": 1, "task": "T1", "prompt": "x"}),
        );
        assert_eq!(a, b, "key order must not change identity");
        let c = Ledger::op_id(kind::AGENT_CALL, "spec-1", Some("T2"), &payload());
        assert_ne!(a, c);
        assert!(a.starts_with("op-") && a.len() == 23);
    }

    #[test]
    fn start_is_idempotent_and_keeps_first_row() {
        with_ledger(|led| {
            let id1 = led
                .start(kind::AGENT_CALL, "spec-1", Some("T1"), &payload(), "t0")
                .unwrap();
            led.set_result(&id1, "runs/spec-1/T1.json").unwrap();
            let id2 = led
                .start(kind::AGENT_CALL, "spec-1", Some("T1"), &payload(), "t1")
                .unwrap();
            assert_eq!(id1, id2);
            let op = led.get(&id1).unwrap().unwrap();
            assert_eq!(op.state, "started");
            assert_eq!(
                op.result_ref.as_deref(),
                Some("runs/spec-1/T1.json"),
                "re-INSERT must not wipe result_ref"
            );
        });
    }

    #[test]
    fn unfinished_only_returns_open_started_ops() {
        with_ledger(|led| {
            let a = led
                .start(
                    kind::SHELL_EXEC,
                    "spec-1",
                    Some("T1"),
                    &json!({"cmd": "cargo check"}),
                    "t",
                )
                .unwrap();
            let b = led
                .start(
                    kind::SHELL_EXEC,
                    "spec-1",
                    Some("T2"),
                    &json!({"cmd": "cargo test"}),
                    "t",
                )
                .unwrap();
            led.finish(&b, None, "t2").unwrap();
            let open = led.unfinished("spec-1").unwrap();
            assert_eq!(
                open.iter().map(|o| o.op_id.as_str()).collect::<Vec<_>>(),
                vec![a.as_str()]
            );
            assert!(led.is_finished(&b).unwrap());
            assert!(!led.is_finished(&a).unwrap());
            assert!(led.unfinished("spec-OTHER").unwrap().is_empty());
        });
    }

    #[test]
    fn plan_resume_covers_every_section_7_4_branch() {
        let mk = |kind: &str, state: &str, result: Option<&str>| Op {
            op_id: "op-x".into(),
            kind: kind.into(),
            spec_id: "s".into(),
            task_id: Some("T1".into()),
            payload_hash: "h".into(),
            state: state.into(),
            result_ref: result.map(|s| s.into()),
        };
        // agent_call, work finished, persistence cut → finalize, never re-call
        assert_eq!(
            plan_resume(&mk(kind::AGENT_CALL, "started", Some("r"))),
            ResumeAction::FinalizeResult
        );
        // agent_call without result → retry with the same input hash
        assert_eq!(
            plan_resume(&mk(kind::AGENT_CALL, "started", None)),
            ResumeAction::ReRun
        );
        // re-runnable tools
        assert_eq!(
            plan_resume(&mk(kind::SHELL_EXEC, "started", Some("r"))),
            ResumeAction::ReRun
        );
        assert_eq!(
            plan_resume(&mk(kind::GIT_OP, "started", None)),
            ResumeAction::ReRun
        );
        assert_eq!(
            plan_resume(&mk(kind::WEB_FETCH, "started", None)),
            ResumeAction::ReRun
        );
        // closed rows are closed
        assert_eq!(
            plan_resume(&mk(kind::AGENT_CALL, "finished", Some("r"))),
            ResumeAction::Done
        );
        assert_eq!(
            plan_resume(&mk(kind::AGENT_CALL, "failed", None)),
            ResumeAction::Done
        );
    }

    #[test]
    fn fail_closes_the_row_and_resume_treats_it_as_closed() {
        with_ledger(|led| {
            let id = led
                .start(
                    kind::AGENT_CALL,
                    "spec-1",
                    Some("T9"),
                    &json!({"i": 9}),
                    "t",
                )
                .unwrap();
            led.fail(&id, "t2").unwrap();
            let op = led.get(&id).unwrap().unwrap();
            assert_eq!(op.state, "failed");
            assert_eq!(plan_resume(&op), ResumeAction::Done);
            assert!(!led.is_finished(&id).unwrap());
        });
    }

    #[test]
    fn payload_hash_column_matches_canonical_hash() {
        with_ledger(|led| {
            let p = json!({"b": 2, "a": 1});
            let id = led.start(kind::WEB_FETCH, "spec-1", None, &p, "t").unwrap();
            let op = led.get(&id).unwrap().unwrap();
            assert_eq!(op.payload_hash, hash::hash_json(&json!({"a": 1, "b": 2})));
        });
    }
}
