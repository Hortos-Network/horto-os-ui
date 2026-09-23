mod state;

#[path = "logs.rs"]
mod component;

pub use component::{
    app_log_error, app_log_info, app_log_lines, app_log_warn, boot_logs, infer_app_log_level,
    LogsPanel,
};
pub use state::LogsState;
