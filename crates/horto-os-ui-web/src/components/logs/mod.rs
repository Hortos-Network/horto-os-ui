mod state;

#[path = "logs.rs"]
mod component;

pub use component::{app_log_error, app_log_info, app_log_lines, boot_logs, LogsPanel};
pub use state::LogsState;
