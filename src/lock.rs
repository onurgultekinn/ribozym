//! Single-copy run lock (§7.5). One `ribozym` writer at a time.
//!
//! The lock is `.ribozym/lock`, holding the holder's pid and a heartbeat
//! mtime. Acquisition is atomic (`create_new`), so two copies racing to start
//! cannot both win. A crashed holder stops heartbeating; once its mtime is
//! older than `stale_after` the next acquirer RECOVERS the lock (§7.5:
//! "çökmüş kilit otomatik kurtarılır") and the caller surfaces a `LockStolen`
//! event so the takeover is auditable, never silent.
//!
//! pid-liveness is inferred from the heartbeat rather than an OS query to stay
//! dependency-free and identical across Windows/Linux CI (a stopped heartbeat
//! is exactly the observable that a crashed process leaves behind).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::clock;
use crate::error::Result;

/// How long a lock may go without a heartbeat before it is deemed crashed.
/// Must comfortably exceed the heartbeat cadence the holder keeps.
pub const DEFAULT_STALE_AFTER: Duration = Duration::from_secs(30);
/// Heartbeat cadence a live holder should refresh at (caller's responsibility).
pub const DEFAULT_HEARTBEAT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub enum AcquireError {
    /// Another live copy holds the lock. Carries its pid for the message.
    HeldBy { pid: u32, age: Duration },
    /// Unrecoverable I/O (permissions, disk).
    Io(String),
}

impl std::fmt::Display for AcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AcquireError::HeldBy { pid, age } => write!(
                f,
                "another ribozym copy (pid {pid}) holds the lock (heartbeat {}s ago)",
                age.as_secs()
            ),
            AcquireError::Io(m) => write!(f, "lock I/O error: {m}"),
        }
    }
}

#[derive(Debug)]
pub struct RunLock {
    path: PathBuf,
    pid: u32,
}

impl RunLock {
    /// Attempt to take the lock. On `HeldBy` a live copy exists — the caller
    /// must abort startup. Returns `(lock, stolen)` where `stolen=true` means
    /// a crashed predecessor's lock was recovered.
    pub fn acquire(
        ribozym_dir: &Path,
        stale_after: Duration,
    ) -> std::result::Result<(Self, bool), AcquireError> {
        std::fs::create_dir_all(ribozym_dir).map_err(|e| AcquireError::Io(e.to_string()))?;
        let path = ribozym_dir.join("lock");
        let pid = std::process::id();

        match Self::try_create(&path, pid) {
            Ok(()) => return Ok((Self { path, pid }, false)),
            // Already exists -> fall through to liveness analysis below.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            // Any other I/O error (permissions, disk) is fatal, not contention.
            Err(e) => return Err(AcquireError::Io(e.to_string())),
        }

        // Lock file exists: is the holder still alive (fresh heartbeat)?
        let held_pid = Self::read_pid(&path);
        let age = Self::age_secs(&path);
        if age < stale_after && held_pid != Some(pid) {
            return Err(AcquireError::HeldBy {
                pid: held_pid.unwrap_or(0),
                age,
            });
        }
        // Crashed (stale) or self-reentrant: remove and retake atomically.
        let _ = std::fs::remove_file(&path);
        Self::try_create(&path, pid).map_err(|_| AcquireError::HeldBy {
            pid: held_pid.unwrap_or(0),
            age,
        })?;
        Ok((Self { path, pid }, true))
    }

    fn try_create(path: &Path, pid: u32) -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        // pid + taken-at (diagnostic; the mtime is the authoritative heartbeat).
        writeln!(f, "{pid} {}", clock::now_utc_iso8601())?;
        f.flush()?;
        f.sync_all()
    }

    /// Refresh the mtime to signal liveness; call once per `heartbeat` tick.
    pub fn touch(&self) -> Result<()> {
        let now = SystemTime::now();
        let f = std::fs::OpenOptions::new().append(true).open(&self.path)?;
        f.set_times(std::fs::FileTimes::new().set_modified(now))?;
        Ok(())
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    fn read_pid(path: &Path) -> Option<u32> {
        let text = std::fs::read_to_string(path).ok()?;
        text.split_whitespace().next()?.parse().ok()
    }

    fn age_secs(path: &Path) -> Duration {
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .map(|m| {
                SystemTime::now()
                    .duration_since(m)
                    .unwrap_or(Duration::ZERO)
            })
            .unwrap_or(Duration::ZERO)
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        // Only remove OUR lock: a stolen-and-reacquired file (same name) must
        // not be unlinked by the old holder's drop.
        if let Ok(existing) = std::fs::read_to_string(&self.path) {
            let same = existing
                .split_whitespace()
                .next()
                .and_then(|p| p.parse::<u32>().ok())
                == Some(self.pid);
            if same {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread::sleep;

    fn touch_file(path: &Path, old_secs: u64) {
        // Backdate mtime to simulate a crashed holder.
        let old = SystemTime::now() - Duration::from_secs(old_secs);
        let f = fs::OpenOptions::new().append(true).open(path).unwrap();
        f.set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();
    }

    #[test]
    fn acquire_succeeds_on_fresh_dir() {
        let dir = tempfile::tempdir().unwrap();
        let (lock, stolen) = RunLock::acquire(dir.path(), DEFAULT_STALE_AFTER).unwrap();
        assert!(!stolen);
        assert_eq!(lock.pid(), std::process::id());
        assert!(dir.path().join("lock").is_file());
    }

    #[test]
    fn second_acquire_is_refused_while_held_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let (lock, _) = RunLock::acquire(dir.path(), DEFAULT_STALE_AFTER).unwrap();
        // Simulate ANOTHER live pid holding the same file.
        {
            let path = dir.path().join("lock");
            fs::remove_file(&path).unwrap();
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            writeln!(f, "999999 {}", clock::now_utc_iso8601()).unwrap();
        }
        let res = RunLock::acquire(dir.path(), DEFAULT_STALE_AFTER);
        assert!(
            matches!(res, Err(AcquireError::HeldBy { pid: 999999, .. })),
            "fresh foreign lock must be refused: {res:?}"
        );
        // keep `lock` alive to Drop cleanly
        drop(lock);
    }

    #[test]
    fn crashed_stale_lock_is_recovered() {
        let dir = tempfile::tempdir().unwrap();
        let (first, _) = RunLock::acquire(dir.path(), Duration::from_secs(1)).unwrap();
        // Foreign, backdated beyond stale_after.
        drop(first);
        let path = dir.path().join("lock");
        {
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            writeln!(f, "888888 {}", clock::now_utc_iso8601()).unwrap();
        }
        touch_file(&path, 60); // 60s old > 1s stale
        let (second, stolen) = RunLock::acquire(dir.path(), DEFAULT_STALE_AFTER).unwrap();
        assert!(
            stolen,
            "stale foreign lock must be recovered with a stolen=true signal"
        );
        assert_eq!(second.pid(), std::process::id());
    }

    #[test]
    fn drop_removes_the_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        let (lock, _) = RunLock::acquire(dir.path(), DEFAULT_STALE_AFTER).unwrap();
        assert!(dir.path().join("lock").exists());
        drop(lock); // Drop must unlink its own lock
        assert!(!dir.path().join("lock").exists());
    }

    #[test]
    fn touch_refreshes_heartbeat_and_ages_back() {
        let dir = tempfile::tempdir().unwrap();
        let (lock, _) = RunLock::acquire(dir.path(), DEFAULT_STALE_AFTER).unwrap();
        let path = dir.path().join("lock");
        touch_file(&path, 20);
        assert!(RunLock::age_secs(&path) > Duration::from_secs(5));
        sleep(Duration::from_millis(15));
        lock.touch().unwrap();
        assert!(
            RunLock::age_secs(&path) < Duration::from_secs(5),
            "touch must move mtime back to now"
        );
    }

    #[test]
    fn reentrant_same_pid_recovers_its_own_stale_lock() {
        let dir = tempfile::tempdir().unwrap();
        let (l1, _) = RunLock::acquire(dir.path(), Duration::from_secs(1)).unwrap();
        // same pid already holds it; force it stale, then re-acquire with same pid
        let path = dir.path().join("lock");
        touch_file(&path, 30);
        let (l2, stolen) = RunLock::acquire(dir.path(), DEFAULT_STALE_AFTER).unwrap();
        assert!(stolen);
        assert_eq!(l1.pid(), l2.pid());
    }
}
