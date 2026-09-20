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
