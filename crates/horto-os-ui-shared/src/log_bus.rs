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

const DEFAULT_CAPACITY: usize = 3000;

/// One log line stored for the Desktop Logs tab.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogEntry {
    /// Monotonic sequence within this process.
    pub seq: u64,
    /// Unix epoch milliseconds when the line was recorded.
    pub ts_ms: u64,
    /// Uppercase level label (`ERROR`, `WARN`, `INFO`, `DEBUG`, `TRACE`).
    pub level: String,
    /// Tracing target or UI source (`connection`, …).
    pub target: String,
    /// Human message body.
    pub message: String,
}

type EmitFn = Arc<dyn Fn(LogEntry) + Send + Sync>;

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
        let message = message.into();
        if message.is_empty() {
            return;
        }
        let entry = LogEntry {
            seq: self.inner.next_seq.fetch_add(1, Ordering::Relaxed),
            ts_ms: now_ms(),
            level: level.to_owned(),
            target: target.to_owned(),
            message,
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
        if message.is_empty() {
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
        if field.name() == "message" {
            self.message = format!("{value:?}");
            if self.message.starts_with('"')
                && self.message.ends_with('"')
                && self.message.len() >= 2
            {
                self.message = self.message[1..self.message.len() - 1]
                    .replace("\\\"", "\"")
                    .replace("\\n", "\n");
            }
        } else if self.message.is_empty() {
            // Keep first non-message field as fallback body.
            self.message = format!("{}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.message);
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
    fn push_lines_skips_blank() {
        let bus = LogBus::new(10);
        bus.push_lines("WARN", "connection", "one\n\ntwo\n");
        let list = bus.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].message, "one");
        assert_eq!(list[1].target, "connection");
    }
}
