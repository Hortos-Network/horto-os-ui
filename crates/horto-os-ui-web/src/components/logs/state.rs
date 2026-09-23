//! App-owned Logs tab state (survives panel remount).

use leptos::prelude::*;

/// One row in the Logs stream (mirrors Desktop [`LogEntry`] JSON).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRow {
    /// Monotonic sequence.
    pub seq: u64,
    /// Unix epoch ms.
    pub ts_ms: u64,
    /// `ERROR` / `WARN` / `INFO` / `DEBUG` / `TRACE`.
    pub level: String,
    /// Tracing target or UI source (`horto`).
    pub target: String,
    /// Message body.
    pub message: String,
}

/// Shared Logs signals.
#[derive(Clone, Copy)]
pub struct LogsState {
    /// Ring snapshot + live appends.
    pub entries: RwSignal<Vec<LogRow>>,
    /// Level filter: `ALL` or a level name.
    pub level_filter: RwSignal<String>,
    /// Case-insensitive substring filter.
    pub search: RwSignal<String>,
    /// Stick stream to bottom on new lines.
    pub follow: RwSignal<bool>,
    /// True after hydrate/listen attached once.
    booted: StoredValue<bool>,
}

impl LogsState {
    /// Fresh empty state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: RwSignal::new(Vec::new()),
            level_filter: RwSignal::new("ALL".into()),
            search: RwSignal::new(String::new()),
            follow: RwSignal::new(true),
            booted: StoredValue::new(false),
        }
    }

    /// Whether boot hydrate/listen already ran.
    #[must_use]
    pub fn is_booted(self) -> bool {
        self.booted.get_value()
    }

    /// Mark boot done.
    pub fn mark_booted(self) {
        self.booted.set_value(true);
    }

    /// Append if `seq` is new (dedupe live + hydrate overlap).
    pub fn push_row(self, row: LogRow) {
        self.entries.update(|rows| {
            if rows.last().is_some_and(|r| r.seq >= row.seq)
                && rows.iter().any(|r| r.seq == row.seq)
            {
                return;
            }
            rows.push(row);
            const CAP: usize = 3000;
            if rows.len() > CAP {
                let drop_n = rows.len() - CAP;
                rows.drain(0..drop_n);
            }
        });
    }

    /// Replace ring contents (hydrate).
    pub fn replace_all(self, rows: Vec<LogRow>) {
        self.entries.set(rows);
    }
}

impl Default for LogsState {
    fn default() -> Self {
        Self::new()
    }
}
