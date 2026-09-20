//! Process-wide `tracing-subscriber` install for Horto binaries.

use std::io;

/// Install a fmt subscriber on stderr.
///
/// Reads `RUST_LOG` when set; otherwise uses `default_filter` (e.g. `"info"`).
/// Later calls are no-ops (`try_init`).
///
/// TUI must not call this: step output belongs in the Logs pane via
/// [`crate::HostContext::logs`], not a stderr fmt layer.
pub fn init_tracing(default_filter: &str) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .try_init();
}

#[cfg(test)]
mod tests {
    #[test]
    fn init_tracing_installs_and_second_call_is_noop() {
        super::init_tracing("info");
        // Global subscriber already set; must not panic.
        super::init_tracing("debug");
    }
}
