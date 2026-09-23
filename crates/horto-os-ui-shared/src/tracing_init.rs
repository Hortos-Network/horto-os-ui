//! Process-wide `tracing-subscriber` install for Horto binaries.

use crate::log_bus::LogBus;
use std::io;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Install a fmt subscriber on stderr.
///
/// Reads `RUST_LOG` when set; otherwise uses `default_filter` (e.g. `"info"`).
/// Later calls are no-ops (`try_init`).
///
/// TUI must not call this: step output belongs in the Logs pane via
/// [`crate::HostContext::logs`], not a stderr fmt layer.
pub fn init_tracing(default_filter: &str) {
    let filter = env_filter(default_filter);
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .try_init();
}

/// Install stderr fmt plus a [`LogBus`] ring layer (Desktop Logs tab).
///
/// Later calls are no-ops. Prefer calling once before any `tracing` macros.
pub fn init_tracing_with_bus(default_filter: &str, bus: &LogBus) {
    let filter = env_filter(default_filter);
    let fmt_layer = tracing_subscriber::fmt::layer().with_writer(io::stderr);
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(bus.layer())
        .try_init();
}

fn env_filter(default_filter: &str) -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter))
}

#[cfg(test)]
mod tests {
    #[test]
    fn init_tracing_installs_and_second_call_is_noop() {
        super::init_tracing("info");
        // Global subscriber already set; must not panic.
        super::init_tracing("debug");
    }

    #[test]
    fn init_with_bus_is_noop_after_fmt() {
        let bus = crate::log_bus::LogBus::with_default_capacity();
        // Either order must not panic (global already may be set by prior test).
        super::init_tracing_with_bus("info", &bus);
        super::init_tracing_with_bus("debug", &bus);
    }
}
