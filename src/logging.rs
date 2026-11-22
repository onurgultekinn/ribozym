//! §15 structured logging — a MINIMAL hand-rolled `tracing::Subscriber`.
//!
//! Why hand-rolled: `tracing-subscriber`/`tracing-serde` are not fetchable in
//! the offline registry this repo builds against, and the dev-dep minimalism
//! rule (Notlar 3) forbids bloating the tree for one appender. `tracing` itself
//! is already a dependency (plan §4), so a ~100-line subscriber that renders
//! WARN-and-above events as one-line `ts|LEVEL|target|msg k=v` records fully
//! satisfies §15 ("run'a gitmeyen, log'a görünen" telemetry) — the rule-4 skip,
//! the local→cloud fallback and the E0 retries finally LAND somewhere.
//!
//! The sink is injectable (`Write`), so tests assert on captured output without
//! touching stderr or the global default.

use std::fmt::Write as _;
use std::io::Write;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Metadata, Subscriber};

/// WARN + ERROR → one line. INFO/DEBUG/TRACE are dropped at the callsite
/// (max_level_hint) so hot paths pay nothing when quiet.
pub const DEFAULT_MAX_LEVEL: Level = Level::WARN;

#[derive(Clone)]
pub struct LineLogger {
    sink: Arc<Mutex<dyn Write + Send>>,
    max: Level,
}

impl LineLogger {
    /// Build a logger writing one-line records into `sink`; returns the
    /// subscriber plus a cloneable handle for flushing/inspection in tests.
    pub fn new(sink: impl Write + Send + 'static) -> Self {
        Self::with_max_level(sink, DEFAULT_MAX_LEVEL)
    }

    pub fn with_max_level(sink: impl Write + Send + 'static, max: Level) -> Self {
        Self {
            sink: Arc::new(Mutex::new(sink)),
            max,
        }
    }

    /// stderr logger for the CLI (§15: visible to the operator, never to the
    /// run's event stream — events.jsonl stays byte-untouched by logging).
    pub fn stderr() -> Self {
        Self::new(std::io::stderr())
    }

    /// Install as this thread's default subscriber (tests use
    /// `tracing::dispatcher::with_default` directly). Returns Err if a global
    /// default already exists — the CLI treats that as "already initialised".
    pub fn install(self) -> Result<(), tracing::subscriber::SetGlobalDefaultError> {
        tracing::subscriber::set_global_default(self)
    }
}

impl Subscriber for LineLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= &self.max
    }

    fn event(&self, event: &Event<'_>) {
        let meta = event.metadata();
        if meta.level() > &self.max {
            return;
        }
        let mut fmt = Line::default();
        event.record(&mut fmt);
        let stamp = crate::clock::now_utc_iso8601();
        let line = format!("{stamp}|{}|{}|{fmt}\n", meta.level(), meta.target());
        if let Ok(mut w) = self.sink.lock() {
            // Logging must never fail a run: a dead sink is swallowed (§15 is
            // telemetry, not a state transition).
            let _ = w.write_all(line.as_bytes());
            let _ = w.flush();
        }
    }

    // Spans are not modelled by this logger: events carry their own spec/task
    // fields, which is all §15 requires for a run log. All span hooks are
    // inert — the four required members exist only to satisfy the trait.
    fn new_span(&self, _attrs: &Attributes<'_>) -> Id {
        Id::from_u64(0)
    }
    fn record(&self, _span: &Id, _values: &Record<'_>) {}
    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}
    fn enter(&self, _span: &Id) {}
    fn exit(&self, _span: &Id) {}
}

/// Renders one event as `message k=v k=v` (message first, fields after, in
/// registration order — deterministic, single-line).
#[derive(Default)]
struct Line {
    message: String,
    fields: String,
}

impl std::fmt::Display for Line {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.fields.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "{}{}", self.message, self.fields)
        }
    }
}

impl Visit for Line {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            // DisplayValue's Debug forwards to Display — no extra quotes.
            let _ = write!(&mut self.message, "{value:?}");
        } else {
            let _ = write!(&mut self.fields, " {}={value:?}", field.name());
        }
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            let _ = write!(&mut self.fields, " {}={}", field.name(), value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warn_events_render_one_line_with_fields() {
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let logger = LineLogger::new(Sink(buf.clone()));
        let dispatch = tracing::Dispatch::new(logger);
        tracing::dispatcher::with_default(&dispatch, || {
            tracing::warn!(task = "T1", op = "op-9", "ledger says finished — skipping");
            tracing::error!("bad");
            tracing::info!("should be dropped");
        });
        let captured = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = captured.trim().lines().collect();
        assert_eq!(lines.len(), 2, "info must be filtered: {lines:?}");
        assert!(lines[0].contains("|WARN|"), "{lines:?}");
        assert!(
            lines[0].contains("ledger says finished — skipping"),
            "{lines:?}"
        );
        assert!(lines[0].contains("task=T1"), "{lines:?}");
        assert!(lines[0].contains("op=op-9"), "{lines:?}");
        assert!(lines[1].contains("|ERROR|"), "{lines:?}");
        // prefix is the §7.3-style stamp: 20 chars, ends in Z
        let stamp = lines[0].split('|').next().unwrap();
        assert_eq!(stamp.len(), 20, "{stamp:?}");
        assert!(stamp.ends_with('Z'), "{stamp:?}");
    }

    #[test]
    fn events_above_max_level_are_filtered() {
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let logger = LineLogger::with_max_level(Sink(buf.clone()), Level::ERROR);
        let dispatch = tracing::Dispatch::new(logger);
        tracing::dispatcher::with_default(&dispatch, || {
            tracing::warn!("quiet");
            tracing::error!("loud");
        });
        let captured = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert_eq!(captured.lines().count(), 1, "{captured}");
        assert!(captured.contains("loud"));
    }

    #[test]
    fn logger_clone_shares_the_sink() {
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let a = LineLogger::new(Sink(buf.clone()));
        let b = a.clone();
        let dispatch = tracing::Dispatch::new(b);
        tracing::dispatcher::with_default(&dispatch, || {
            tracing::warn!("via clone");
        });
        let captured = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(captured.contains("via clone"));
        let _ = a;
    }

    /// Test adapter: a `Write` that appends into a shared Vec.
    struct Sink(Arc<Mutex<Vec<u8>>>);
    impl Write for Sink {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
