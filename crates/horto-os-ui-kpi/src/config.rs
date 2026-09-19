//! KPI board poll / history settings.

/// Live board settings (fixed 3x3 chart grid).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardConfig {
    pub history: usize,
    pub poll_secs: u64,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            history: 60,
            poll_secs: 2,
        }
    }
}
