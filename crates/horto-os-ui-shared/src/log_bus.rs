//! In-process ring buffer for Desktop app logs (tracing + UI appends).

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

/// Default ring size for Desktop ([`LogBus::with_default_capacity`]).
pub const DEFAULT_CAPACITY: usize = 3000;

/// One log line stored for the Desktop Logs tab.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogEntry {
    /// Monotonic sequence within this process.
    pub seq: u64,
    /// Unix epoch milliseconds when the line was recorded.
    pub ts_ms: u64,
    /// Uppercase level label (`ERROR`, `WARN`, `INFO`, `DEBUG`, `TRACE`).
    pub level: String,
    /// Tracing target or UI source (`horto`, …).
    pub target: String,
    /// Human message body.
    pub message: String,
}

type EmitFn = Arc<dyn Fn(LogEntry) + Send + Sync>;

/// Optional Desktop process bus so remote Capture can append at the right time
/// (after apply SSH, before payload banners) without going through the UI return path.
static PROCESS_BUS: Mutex<Option<LogBus>> = Mutex::new(None);

/// Shared ring of [`LogEntry`] values for Desktop.
#[derive(Clone)]
pub struct LogBus {
    inner: Arc<LogBusInner>,
}

struct LogBusInner {
    capacity: usize,
    next_seq: AtomicU64,
    entries: Mutex<VecDeque<LogEntry>>,
    emit: Mutex<Option<EmitFn>>,
}

impl LogBus {
    /// Create a bus that keeps at most `capacity` entries (oldest dropped).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(LogBusInner {
                capacity: capacity.max(1),
                next_seq: AtomicU64::new(1),
                entries: Mutex::new(VecDeque::new()),
                emit: Mutex::new(None),
            }),
        }
    }

    /// Default Desktop capacity ([`DEFAULT_CAPACITY`]).
    #[must_use]
    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }

    /// Register this bus as the process-wide Desktop sink (idempotent replace).
    pub fn install_as_process_bus(&self) {
        if let Ok(mut slot) = PROCESS_BUS.lock() {
            *slot = Some(self.clone());
        }
    }

    /// Append Capture stdout/stderr into the process bus (no-op when unset / CLI).
    ///
    /// Call after SSH Capture returns and before later PC progress banners so Logs
    /// stay chronological with the terminal `eprintln` dump.
    pub fn mirror_capture_to_process_bus(text: &str) {
        let Ok(slot) = PROCESS_BUS.lock() else {
            return;
        };
        let Some(bus) = slot.as_ref() else {
            return;
        };
        for line in text.lines() {
            let line = line.trim_end();
            if line.is_empty() {
                continue;
            }
            let level = if line.contains(" WARNING")
                || line.contains(" WARN ")
                || line.starts_with("WARNING:")
                || line.contains("): WARNING **:")
            {
                "WARN"
            } else if line.contains(" ERROR") || line.starts_with("error:") {
                "ERROR"
            } else {
                "INFO"
            };
            bus.push(level, "horto.box", line);
        }
    }

    /// Register a live sink (Tauri emit / webview dispatch). Replaces any prior sink.
    pub fn set_emitter<F>(&self, f: F)
    where
        F: Fn(LogEntry) + Send + Sync + 'static,
    {
        if let Ok(mut slot) = self.inner.emit.lock() {
            *slot = Some(Arc::new(f));
        }
    }

    /// Snapshot of current ring contents (oldest first).
    #[must_use]
    pub fn list(&self) -> Vec<LogEntry> {
        self.inner
            .entries
            .lock()
            .map(|g| g.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Drop every stored entry.
    pub fn clear(&self) {
        if let Ok(mut g) = self.inner.entries.lock() {
            g.clear();
        }
    }

    /// Append a line from UI or tracing.
    pub fn push(&self, level: &str, target: &str, message: impl Into<String>) {
        let message = crate::ansi::strip_ansi(&message.into());
        let message = message.trim();
        if message.is_empty() {
            return;
        }
        let entry = LogEntry {
            seq: self.inner.next_seq.fetch_add(1, Ordering::Relaxed),
            ts_ms: now_ms(),
            level: level.to_owned(),
            target: target.to_owned(),
            message: message.to_owned(),
        };
        if let Ok(mut g) = self.inner.entries.lock() {
            while g.len() >= self.inner.capacity {
                g.pop_front();
            }
            g.push_back(entry.clone());
        }
        if let Ok(slot) = self.inner.emit.lock() {
            if let Some(emit) = slot.as_ref() {
                emit(entry);
            }
        }
    }

    /// Append each non-empty line of `text` with the same level/target.
    pub fn push_lines(&self, level: &str, target: &str, text: &str) {
        for line in text.lines() {
            let line = line.trim_end();
            if !line.is_empty() {
                self.push(level, target, line.to_owned());
            }
        }
    }

    /// Tracing layer that records events into this bus.
    #[must_use]
    pub fn layer(&self) -> LogBusLayer {
        LogBusLayer { bus: self.clone() }
    }
}

/// [`Layer`] that copies tracing events into a [`LogBus`].
#[derive(Clone)]
pub struct LogBusLayer {
    bus: LogBus,
}

impl<S> Layer<S> for LogBusLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let message = visitor.message;
        if message.trim().is_empty() {
            return;
        }
        let meta = event.metadata();
        self.bus
            .push(level_label(*meta.level()), meta.target(), message);
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() != "message" {
            return;
        }
        self.message = format!("{value:?}");
        if self.message.starts_with('"') && self.message.ends_with('"') && self.message.len() >= 2 {
            self.message = self.message[1..self.message.len() - 1]
                .replace("\\\"", "\"")
                .replace("\\n", "\n");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.message);
        }
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        if field.name() == "message" {
            self.message = value.to_string();
        }
    }
}

const fn level_label(level: Level) -> &'static str {
    match level {
        Level::ERROR => "ERROR",
        Level::WARN => "WARN",
        Level::INFO => "INFO",
        Level::DEBUG => "DEBUG",
        Level::TRACE => "TRACE",
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::prelude::*;

    #[test]
    fn ring_truncates_oldest() {
        let bus = LogBus::new(3);
        bus.push("INFO", "t", "a");
        bus.push("INFO", "t", "b");
        bus.push("INFO", "t", "c");
        bus.push("INFO", "t", "d");
        let list = bus.list();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].message, "b");
        assert_eq!(list[2].message, "d");
    }

    #[test]
    fn zero_capacity_clamps_to_one() {
        let bus = LogBus::new(0);
        bus.push("INFO", "t", "only");
        bus.push("INFO", "t", "next");
        let list = bus.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].message, "next");
    }

    #[test]
    fn with_default_capacity_matches_constant() {
        let bus = LogBus::with_default_capacity();
        for i in 0..5 {
            bus.push("INFO", "horto", format!("line-{i}"));
        }
        assert_eq!(bus.list().len(), 5);
        assert_eq!(DEFAULT_CAPACITY, 3000);
    }

    #[test]
    fn push_skips_empty_and_clear_empties() {
        let bus = LogBus::new(10);
        bus.push("INFO", "t", "");
        assert_eq!(bus.list().len(), 0);
        bus.push("INFO", "t", "kept");
        bus.clear();
        assert_eq!(bus.list().len(), 0);
    }

    #[test]
    fn push_lines_skips_blank() {
        let bus = LogBus::new(10);
        bus.push_lines("WARN", "horto", "one\n\ntwo\n");
        let list = bus.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].message, "one");
        assert_eq!(list[1].target, "horto");
    }

    #[test]
    fn emitter_receives_appends_and_can_be_replaced() {
        let bus = LogBus::new(10);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_a = Arc::clone(&seen);
        bus.set_emitter(move |e| {
            seen_a.lock().expect("lock").push(e.message);
        });
        bus.push("INFO", "t", "first");
        let seen_b = Arc::new(Mutex::new(Vec::new()));
        let seen_b_c = Arc::clone(&seen_b);
        bus.set_emitter(move |e| {
            seen_b_c.lock().expect("lock").push(e.message);
        });
        bus.push("INFO", "t", "second");
        assert_eq!(seen.lock().expect("lock").as_slice(), ["first"]);
        assert_eq!(seen_b.lock().expect("lock").as_slice(), ["second"]);
    }

    #[test]
    fn entry_seq_and_serde_roundtrip() {
        let bus = LogBus::new(10);
        bus.push("ERROR", "horto", "boom");
        let entry = bus.list().pop().expect("one");
        assert_eq!(entry.seq, 1);
        assert!(entry.ts_ms > 0);
        let json = serde_json::to_string(&entry).expect("ser");
        let back: LogEntry = serde_json::from_str(&json).expect("de");
        assert_eq!(back, entry);
    }

    #[test]
    fn mirror_capture_uses_process_bus_when_installed() {
        let bus = LogBus::new(32);
        bus.install_as_process_bus();
        LogBus::mirror_capture_to_process_bus(
            "ok line\n\nWARNING: noisy\nerror: boom\n** (x): WARNING **: netplan\n",
        );
        let list = bus.list();
        assert_eq!(list.len(), 4);
        assert_eq!(list[0].message, "ok line");
        assert_eq!(list[0].target, "horto.box");
        assert_eq!(list[0].level, "INFO");
        assert_eq!(list[1].level, "WARN");
        assert_eq!(list[2].level, "ERROR");
        assert_eq!(list[3].level, "WARN");
    }

    #[test]
    fn mirror_capture_is_noop_without_process_bus() {
        // Clear any prior install from other tests in this process.
        if let Ok(mut slot) = PROCESS_BUS.lock() {
            *slot = None;
        }
        LogBus::mirror_capture_to_process_bus("orphan line");
        let bus = LogBus::new(8);
        assert_eq!(bus.list().len(), 0);
    }

    #[test]
    fn layer_records_levels_and_message_shapes() {
        let bus = LogBus::new(32);
        let subscriber = tracing_subscriber::registry().with(bus.layer());
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!(target: "horto.test", "err-line");
            tracing::warn!(target: "horto.test", "warn-line");
            tracing::info!(target: "horto.test", "info-line");
            tracing::debug!(target: "horto.test", "debug-line");
            tracing::trace!(target: "horto.test", "trace-line");
            tracing::info!(target: "horto.test", message = ?"debug-quoted");
            tracing::info!(target: "horto.test", message = ?"say \"hi\"\nnext");
            // Structured fields without `message` are skipped (no invented body).
            tracing::info!(target: "horto.test", code = 42);
            tracing::info!(target: "horto.test", other = "ignored");
        });
        let list = bus.list();
        let levels: Vec<_> = list.iter().map(|e| e.level.as_str()).collect();
        assert!(levels.contains(&"ERROR"));
        assert!(levels.contains(&"WARN"));
        assert!(levels.contains(&"INFO"));
        assert!(levels.contains(&"DEBUG"));
        assert!(levels.contains(&"TRACE"));
        assert!(list.iter().any(|e| e.message == "err-line"));
        assert!(list.iter().any(|e| e.message == "debug-quoted"));
        assert!(list.iter().any(|e| e.message == "say \"hi\"\nnext"));
        assert!(!list.iter().any(|e| e.message.starts_with("code=")));
        assert!(!list.iter().any(|e| e.message.is_empty()));
        assert!(!list.iter().any(|e| e.message == "ignored"));
    }
}
