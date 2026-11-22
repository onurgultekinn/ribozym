//! Append-only event log — `runs/<spec_id>/events.jsonl` (§7.3).
//!
//! The event bus is a file, not a queue: every state change is one JSON line
//! with a strictly increasing `seq` within the run (P2: "her durum değişikliği
//! events.jsonl içinde artan seq ile bulunur"). Single writer is the control
//! plane (§7.5); the lock guarantees only one copy appends.
//!
//! Durability (§7.3): each append is flushed AND fsynced, so a `kill` between
//! events cannot lose a recorded transition — resume replays exactly the
//! events that reached disk.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};

use serde_json::Value;

use crate::clock;
use crate::error::{Error, Result};
use crate::models::event::{EventEnvelope, EventKind};
use crate::models::Extra;

const SCHEMA_VERSION: &str = "1.0";

/// A log with the run's identity pinned; `append` stamps seq + ts itself.
pub struct EventLog {
    path: std::path::PathBuf,
    spec_id: String,
    run_id: String,
    last_seq: u64,
    file: File,
}

impl EventLog {
    /// Open (creating) the log for a run and recover the next `seq` by reading
    /// the tail — a re-opened run continues numbering, never restarts at 1.
    pub fn open(path: &std::path::Path, spec_id: &str, run_id: &str) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(path)?;
        let mut log = Self {
            path: path.to_path_buf(),
            spec_id: spec_id.to_string(),
            run_id: run_id.to_string(),
            last_seq: 0,
            file,
        };
        log.last_seq = log.scan_last_seq()?;
        Ok(log)
    }

    /// Read existing lines and return the highest well-formed `seq` for THIS
    /// run. A corrupt trailing line (partial write before a crash) is ignored
    /// with a warning — the next append simply starts after the last good one.
    fn scan_last_seq(&self) -> Result<u64> {
        let mut reader = BufReader::new(File::open(&self.path)?);
        let mut last = 0u64;
        let mut buf = String::new();
        let mut lineno = 0;
        loop {
            buf.clear();
            if reader.read_line(&mut buf)? == 0 {
                break;
            }
            lineno += 1;
            let line = buf.trim_end();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<EventEnvelope>(line) {
                Ok(ev) if ev.run_id == self.run_id => last = last.max(ev.seq),
                Ok(_) => {} // event from a different run id in same file — skip
                Err(e) => {
                    tracing::warn!(line = lineno, error = %e, "skipping unparseable event line on open");
                }
            }
        }
        Ok(last)
    }

    /// Append one event; returns the full envelope (seq/ts assigned here).
    /// The single writer is the control plane; a duplicate-write attempt
    /// (seq collision) is impossible because seq is taken from `last_seq`.
    pub fn append(
        &mut self,
        event: EventKind,
        correlation_id: &str,
        task_id: Option<&str>,
        data: Extra,
    ) -> Result<EventEnvelope> {
        self.last_seq += 1;
        let envelope = EventEnvelope {
            seq: self.last_seq,
            ts: clock::now_utc_iso8601(),
            spec_id: self.spec_id.clone(),
            run_id: self.run_id.clone(),
            task_id: task_id.map(|s| s.to_string()),
            event,
            correlation_id: correlation_id.to_string(),
            schema_version: SCHEMA_VERSION.to_string(),
            data,
        };
        let mut line = serde_json::to_string(&envelope)?;
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        // Sync per event (§7.3): a recorded transition survives kill(2).
        self.file.sync_all()?;
        Ok(envelope)
    }

    /// Convenience: append with a `data` payload built from key/value pairs.
    pub fn append_with(
        &mut self,
        event: EventKind,
        correlation_id: &str,
        task_id: Option<&str>,
        pairs: &[(&str, Value)],
    ) -> Result<EventEnvelope> {
        let mut data = Extra::new();
        for (k, v) in pairs {
            data.insert((*k).to_string(), v.clone());
        }
        self.append(event, correlation_id, task_id, data)
    }

    /// Read back every event for this run in file order (resume + replay).
    pub fn replay(&self) -> Result<Vec<EventEnvelope>> {
        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let mut out = Vec::new();
        for line in reader.lines() {
            let line = line?;
            let line = line.trim_end();
            if line.is_empty() {
                continue;
            }
            let ev: EventEnvelope = serde_json::from_str(line)
                .map_err(|e| Error::ConfigInvalid(format!("corrupt event log line: {e}")))?;
            if ev.run_id == self.run_id {
                out.push(ev);
            }
        }
        Ok(out)
    }

    pub fn next_seq(&self) -> u64 {
        self.last_seq + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp_log(name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let _ = name;
        (dir, path)
    }

    #[test]
    fn seq_starts_at_one_and_increases() {
        let (_d, path) = tmp_log("a");
        let mut log = EventLog::open(&path, "spec-20260912-001", "run-x1").unwrap();
        let e1 = log
            .append_with(EventKind::SpecReceived, "op-1", None, &[("k", json!(1))])
            .unwrap();
        let e2 = log
            .append_with(EventKind::TasksCreated, "op-2", Some("T1"), &[])
            .unwrap();
        assert_eq!((e1.seq, e2.seq), (1, 2));
        assert_eq!(e2.task_id.as_deref(), Some("T1"));
        assert_eq!(e1.data["k"], 1);
    }

    #[test]
    fn reopen_continues_numbering_from_disk() {
        let (_d, path) = tmp_log("b");
        {
            let mut log = EventLog::open(&path, "spec-20260912-001", "run-x1").unwrap();
            log.append(EventKind::SpecReceived, "op-1", None, Extra::new())
                .unwrap();
            log.append(EventKind::SpecValidated, "op-2", None, Extra::new())
                .unwrap();
        }
        // Fresh handle sees last_seq=2, so the third event is seq=3.
        let mut log = EventLog::open(&path, "spec-20260912-001", "run-x1").unwrap();
        let e3 = log
            .append(EventKind::TasksCreated, "op-3", None, Extra::new())
            .unwrap();
        assert_eq!(e3.seq, 3);
        let all = log.replay().unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].seq, 1);
        assert_eq!(all[2].seq, 3);
    }

    #[test]
    fn replay_reports_events_in_order_and_only_this_run() {
        let (_d, path) = tmp_log("c");
        let mut log = EventLog::open(&path, "spec-20260912-001", "run-x1").unwrap();
        for i in 0..5 {
            log.append(
                EventKind::TaskStarted,
                &format!("op-{i}"),
                None,
                Extra::new(),
            )
            .unwrap();
        }
        let evs = log.replay().unwrap();
        let seqs: Vec<u64> = evs.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3, 4, 5]);
        // strictly increasing (P2 invariant, the resume/dup-detection anchor)
        assert!(seqs.windows(2).all(|w| w[1] > w[0]));
    }

    #[test]
    fn corrupt_trailing_line_is_skipped_on_open_not_panic() {
        let (_d, path) = tmp_log("d");
        {
            let mut log = EventLog::open(&path, "spec-20260912-001", "run-x1").unwrap();
            log.append(EventKind::SpecReceived, "op-1", None, Extra::new())
                .unwrap();
        }
        // Simulate a half-written line from a crash mid-append.
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{\"seq\":2,\"ts\":").unwrap();
        // reopen: recovers seq=1 (the good line), ignores the partial one.
        let log = EventLog::open(&path, "spec-20260912-001", "run-x1").unwrap();
        assert_eq!(log.next_seq(), 2);
    }

    #[test]
    fn foreign_run_in_same_file_does_not_advance_seq() {
        let (_d, path) = tmp_log("e");
        {
            let mut log = EventLog::open(&path, "spec-20260912-001", "run-OTHER").unwrap();
            log.append(EventKind::SpecReceived, "op-1", None, Extra::new())
                .unwrap();
        }
        let mut log = EventLog::open(&path, "spec-20260912-001", "run-x1").unwrap();
        let e = log
            .append(EventKind::SpecReceived, "op-1", None, Extra::new())
            .unwrap();
        assert_eq!(e.seq, 1, "run-x1 numbering is independent of run-OTHER");
    }
}
