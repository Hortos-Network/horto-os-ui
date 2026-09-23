mod state;

#[path = "logs.rs"]
mod component;

pub use component::{boot_logs, mirror_connection_log, LogsPanel};
pub use state::LogsState;
