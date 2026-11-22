//! State store (§7). Single global SQLite at `.ribozym/state.db`; single
//! writer is the control plane; WAL + busy_timeout 5s (§7.5).
//!
//! Migrations live in `.ribozym/migrations/*.sql` and are applied in filename
//! order, once each, recorded in `_migrations` — resume auto-applies pending
//! migrations before anything else runs (§11.0).

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::error::Result;

pub struct StateStore {
    conn: Connection,
}

impl StateStore {
    /// Open (creating if needed) and bring the schema up to date.
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    /// In-memory store for tests.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "journal_mode", "MEMORY")?;
        let mut store = Self { conn };
        store.apply_embedded_migrations()?;
        Ok(store)
    }

    fn migrations_dir() -> PathBuf {
        // Migrations travel with the repo, not with the db file; resolve
        // relative to the repo root found from CWD at open time.
        crate::config::find_repo_root(&std::env::current_dir().unwrap_or_default())
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".ribozym")
            .join("migrations")
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS _migrations(
               name TEXT PRIMARY KEY,
               applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
             );",
        )?;
        let dir = Self::migrations_dir();
        let mut pending: Vec<PathBuf> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().is_some_and(|e| e == "sql") {
                    pending.push(p);
                }
            }
        }
        pending.sort();
        for path in pending {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let seen: bool = self.conn.query_row(
                "SELECT COUNT(*) FROM _migrations WHERE name = ?1",
                [&name],
                |r| r.get::<_, i64>(0),
            )? > 0;
            if seen {
                continue;
            }
            let sql = std::fs::read_to_string(&path)?;
            self.conn.execute_batch(&sql)?;
            self.conn
                .execute("INSERT INTO _migrations(name) VALUES (?1)", [&name])?;
        }
        Ok(())
    }

    /// Test helper: applies the migrations embedded at compile time, in
    /// filename order — byte-identical to what the on-disk runner feeds a real
    /// `state.db`, so a schema the tests see can never diverge from `run`.
    #[cfg(test)]
    fn apply_embedded_migrations(&mut self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS _migrations(
               name TEXT PRIMARY KEY,
               applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
             );",
        )?;
        const MIGRATIONS: &[(&str, &str)] = &[
            (
                "0001_init.sql",
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/.ribozym/migrations/0001_init.sql"
                )),
            ),
            (
                "0002_budget_pause.sql",
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/.ribozym/migrations/0002_budget_pause.sql"
                )),
            ),
            (
                "0003_charge_idempotency.sql",
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/.ribozym/migrations/0003_charge_idempotency.sql"
                )),
            ),
        ];
        for (name, sql) in MIGRATIONS {
            self.conn.execute_batch(sql)?;
            self.conn.execute(
                "INSERT OR IGNORE INTO _migrations(name) VALUES (?1)",
                [name],
            )?;
        }
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}
